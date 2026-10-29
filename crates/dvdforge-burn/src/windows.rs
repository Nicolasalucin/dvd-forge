//! Windows IMAPI2 backend.
//!
//! The pure helpers in this module (drive paths, DVD speed conversion,
//! capacity guards and stream progress) are compiled and tested on every
//! platform. COM/IMAPI2 calls are isolated behind `#[cfg(target_os = "windows")]`.

use crate::{
    BurnError, BurnProgress, DiscMediaKind, MediaStatus, DVD_1X_BYTES_PER_SECOND, DVD_SECTOR_BYTES,
};
use std::time::{Duration, Instant};

pub const IMAPI_BLANK_STATE: i32 = 2;
pub const IMAPI_APPENDABLE_STATE: i32 = 4;
pub const IMAPI_WRITE_PROTECTED_STATE: i32 = 8_192;
pub const IMAPI_FINALIZED_STATE: i32 = 16_384;
pub const IMAPI_UNSUPPORTED_MEDIA_STATE: i32 = 32_768;

/// Converts a Windows drive identifier (`D:`, `D`, `\\.\D:`) to the raw volume
/// path used for sector verification.
pub fn raw_volume_path(device: &str) -> Result<String, BurnError> {
    let trimmed = device.trim();
    if trimmed.is_empty() {
        return Err(BurnError::InvalidRequest(
            "el dispositivo Windows no puede estar vacío".to_string(),
        ));
    }
    if trimmed.starts_with("\\\\.\\") || trimmed.starts_with("\\\\?\\") {
        return Ok(trimmed.to_string());
    }
    let mut chars = trimmed.chars();
    let letter = chars
        .next()
        .ok_or_else(|| BurnError::InvalidRequest("dispositivo Windows inválido".to_string()))?;
    if !letter.is_ascii_alphabetic() {
        return Err(BurnError::InvalidRequest(format!(
            "se esperaba una letra de unidad Windows, recibido {device}"
        )));
    }
    if let Some(rest) = chars.next() {
        if rest != ':' || chars.next().is_some() {
            return Err(BurnError::InvalidRequest(format!(
                "ruta de unidad Windows inválida: {device}"
            )));
        }
    }
    Ok(format!("\\\\.\\{}:", letter.to_ascii_uppercase()))
}

/// Returns the canonical `X:` identifier used by the frontend.
pub fn normalize_drive_id(device: &str) -> Result<String, BurnError> {
    let path = raw_volume_path(device)?;
    let letter = path
        .trim_end_matches(':')
        .chars()
        .last()
        .filter(|character| character.is_ascii_alphabetic())
        .ok_or_else(|| BurnError::InvalidRequest(format!("unidad inválida: {device}")))?;
    Ok(format!("{}:", letter.to_ascii_uppercase()))
}

/// IMAPI2 reports write speed in DVD sectors per second (2048-byte sectors).
pub fn sectors_per_second_to_speed_x(sectors: i32) -> f32 {
    sectors_per_second_to_speed_x_for(DiscMediaKind::DvdR, sectors)
}

/// Media-aware variant: 1x is 1385 KB/s on DVD but 4495 KB/s on Blu-ray.
pub fn sectors_per_second_to_speed_x_for(kind: DiscMediaKind, sectors: i32) -> f32 {
    if sectors <= 0 {
        return 0.0;
    }
    (sectors as f64 * DVD_SECTOR_BYTES as f64 / kind.bytes_per_second_at_1x()) as f32
}

pub fn speed_x_to_sectors_per_second(speed_x: f32) -> i32 {
    speed_x_to_sectors_per_second_for(DiscMediaKind::DvdR, speed_x)
}

pub fn speed_x_to_sectors_per_second_for(kind: DiscMediaKind, speed_x: f32) -> i32 {
    if !speed_x.is_finite() || speed_x <= 0.0 {
        return 0;
    }
    (speed_x as f64 * kind.bytes_per_second_at_1x() / DVD_SECTOR_BYTES as f64).round() as i32
}

pub fn imapi_sectors_to_bytes(sectors: i32) -> u64 {
    sectors.max(0) as u64 * DVD_SECTOR_BYTES
}

pub fn imapi_media_kind(value: i32) -> DiscMediaKind {
    match value {
        4 => DiscMediaKind::DvdRom,
        5 => DiscMediaKind::DvdRam,
        6 | 8 => DiscMediaKind::DvdPlusR,
        7 | 13 => DiscMediaKind::DvdPlusRw,
        9 | 11 => DiscMediaKind::DvdR,
        10 => DiscMediaKind::DvdRw,
        // IMAPI_MEDIA_TYPE_BDROM / _BDR / _BDRE.
        17 => DiscMediaKind::BdRom,
        18 => DiscMediaKind::BdR,
        19 => DiscMediaKind::BdRe,
        _ => DiscMediaKind::Unknown,
    }
}

pub fn imapi_media_status(value: i32, kind: DiscMediaKind) -> MediaStatus {
    if value & IMAPI_UNSUPPORTED_MEDIA_STATE != 0 {
        MediaStatus::Unknown
    } else if value & IMAPI_FINALIZED_STATE != 0 {
        MediaStatus::Closed
    } else if value & IMAPI_BLANK_STATE != 0 {
        MediaStatus::Blank
    } else if value & IMAPI_APPENDABLE_STATE != 0 {
        MediaStatus::Appendable
    } else if kind == DiscMediaKind::DvdRom {
        MediaStatus::Complete
    } else {
        MediaStatus::Unknown
    }
}

pub fn imapi_media_writable(value: i32, kind: DiscMediaKind) -> bool {
    kind.writable()
        && value & IMAPI_WRITE_PROTECTED_STATE == 0
        && value & IMAPI_UNSUPPORTED_MEDIA_STATE == 0
}

pub fn stream_progress(written_bytes: u64, total_bytes: u64, elapsed: Duration) -> BurnProgress {
    let written = written_bytes.min(total_bytes);
    let percent = if total_bytes == 0 {
        0.0
    } else {
        (written as f64 / total_bytes as f64 * 100.0).clamp(0.0, 100.0) as f32
    };
    let seconds = elapsed.as_secs_f64();
    let bytes_per_second = if seconds > 0.0 {
        written as f64 / seconds
    } else {
        0.0
    };
    let speed_x = if bytes_per_second > 0.0 {
        Some((bytes_per_second / DVD_1X_BYTES_PER_SECOND) as f32)
    } else {
        None
    };
    let eta_secs = if bytes_per_second > 0.0 && written < total_bytes {
        Some(((total_bytes - written) as f64 / bytes_per_second).ceil() as u64)
    } else {
        None
    };
    BurnProgress {
        written_bytes: written,
        total_bytes,
        percent,
        speed_x,
        eta_secs,
    }
}

#[derive(Debug)]
pub struct StreamProgressTracker {
    total_bytes: u64,
    started: Instant,
    written: u64,
}

impl StreamProgressTracker {
    pub fn new(total_bytes: u64) -> Self {
        Self {
            total_bytes,
            started: Instant::now(),
            written: 0,
        }
    }

    pub fn record(&mut self, bytes: usize) -> BurnProgress {
        self.written = self.written.saturating_add(bytes as u64);
        stream_progress(self.written, self.total_bytes, self.started.elapsed())
    }

    pub fn written_bytes(&self) -> u64 {
        self.written
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::*;
    use crate::{
        BurnBackend, BurnReport, BurnRequest, DeviceId, DiscBurner, MediaInfo, OpticalDevice,
        ProgressSink, VerifyReport,
    };
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio_util::sync::CancellationToken;
    use windows::core::{implement, Ref, Result as WinResult, BSTR, HRESULT};
    use windows::Win32::Foundation::{
        E_ABORT, E_FAIL, E_NOTIMPL, E_POINTER, VARIANT_FALSE, VARIANT_TRUE,
    };
    use windows::Win32::Storage::Imapi::{
        IDiscFormat2Data, IDiscMaster2, IDiscRecorder2, MsftDiscFormat2Data, MsftDiscMaster2,
        MsftDiscRecorder2,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, ISequentialStream_Impl, IStream,
        IStream_Impl, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, LOCKTYPE, SAFEARRAY,
        STATFLAG, STATSTG, STGC, STGM_READ, STGTY_STREAM, STREAM_SEEK, STREAM_SEEK_CUR,
        STREAM_SEEK_END, STREAM_SEEK_SET,
    };
    use windows::Win32::System::Ole::{
        SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound,
        SafeArrayUnaccessData,
    };

    #[derive(Debug, Default, Clone, Copy)]
    pub struct WindowsBurner;

    impl WindowsBurner {
        pub fn new() -> Self {
            Self
        }
    }

    struct ComApartment;

    impl ComApartment {
        fn new() -> Result<Self, BurnError> {
            unsafe {
                CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                    .ok()
                    .map_err(map_com)?;
            }
            Ok(Self)
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    fn map_com(error: windows::core::Error) -> BurnError {
        BurnError::Imapi(error.message())
    }

    fn map_join(error: tokio::task::JoinError) -> BurnError {
        BurnError::Imapi(format!("tarea COM terminó inesperadamente: {error}"))
    }

    /// Drive letter of a recorder, from its mount points (`D:\`).
    /// `LegacyDeviceNumber` is the N of `\\.\CdRomN`, not a letter index.
    fn drive_letter(recorder: &IDiscRecorder2) -> Option<char> {
        let paths = unsafe { recorder.VolumePathNames() }.ok()?;
        if paths.is_null() {
            return None;
        }
        let letter = unsafe { first_drive_letter(paths) };
        let _ = unsafe { SafeArrayDestroy(paths) };
        letter
    }

    /// Scans a SAFEARRAY of `VT_BSTR` VARIANTs for the first `X:` mount point.
    unsafe fn first_drive_letter(paths: *mut SAFEARRAY) -> Option<char> {
        const VT_BSTR: u16 = 8;
        let lower = unsafe { SafeArrayGetLBound(paths, 1) }.ok()?;
        let upper = unsafe { SafeArrayGetUBound(paths, 1) }.ok()?;
        let stride = unsafe { (*paths).cbElements } as usize;
        let mut data = std::ptr::null_mut();
        unsafe { SafeArrayAccessData(paths, &mut data) }.ok()?;
        let mut letter = None;
        for index in 0..usize::try_from(upper - lower + 1).unwrap_or(0) {
            // VARIANT layout: u16 `vt`, 6 reserved bytes, then the BSTR pointer.
            let element = unsafe { data.cast::<u8>().add(index * stride) };
            if unsafe { element.cast::<u16>().read_unaligned() } != VT_BSTR {
                continue;
            }
            let text = unsafe { element.add(8).cast::<*const u16>().read_unaligned() };
            // The BSTR length prefix (bytes) sits just before the text.
            if text.is_null()
                || unsafe { text.cast::<u8>().sub(4).cast::<u32>().read_unaligned() } < 4
            {
                continue;
            }
            let (first, second) = unsafe { (text.read_unaligned(), text.add(1).read_unaligned()) };
            if second == u16::from(b':') && first < 0x80 && (first as u8).is_ascii_alphabetic() {
                letter = Some((first as u8).to_ascii_uppercase() as char);
                break;
            }
        }
        let _ = unsafe { SafeArrayUnaccessData(paths) };
        letter
    }

    fn com_list_devices() -> Result<Vec<OpticalDevice>, BurnError> {
        let _com = ComApartment::new()?;
        let master: IDiscMaster2 = unsafe {
            CoCreateInstance(
                &MsftDiscMaster2,
                None::<&windows::core::IUnknown>,
                CLSCTX_INPROC_SERVER,
            )
        }
        .map_err(map_com)?;
        let count = unsafe { master.Count() }.map_err(map_com)?;
        let mut devices = Vec::new();
        for index in 0..count {
            let unique_id = unsafe { master.get_Item(index) }.map_err(map_com)?;
            let recorder: IDiscRecorder2 = unsafe {
                CoCreateInstance(
                    &MsftDiscRecorder2,
                    None::<&windows::core::IUnknown>,
                    CLSCTX_INPROC_SERVER,
                )
            }
            .map_err(map_com)?;
            if unsafe { recorder.InitializeDiscRecorder(&unique_id) }.is_err() {
                continue;
            }
            let Some(letter) = drive_letter(&recorder) else {
                continue;
            };
            let vendor = unsafe { recorder.VendorId() }
                .map(|value| value.to_string())
                .unwrap_or_default();
            let model = unsafe { recorder.ProductId() }
                .map(|value| value.to_string())
                .unwrap_or_default();
            devices.push(OpticalDevice {
                id: DeviceId::new(format!("{letter}:")),
                vendor: vendor.trim().to_string(),
                model: model.trim().to_string(),
                bus: "imapi".to_string(),
                writable: true,
                media_present: false,
            });
        }
        devices.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        Ok(devices)
    }

    fn find_recorder(device: &str) -> Result<(IDiscRecorder2, String), BurnError> {
        let normalized = normalize_drive_id(device)?;
        let master: IDiscMaster2 = unsafe {
            CoCreateInstance(
                &MsftDiscMaster2,
                None::<&windows::core::IUnknown>,
                CLSCTX_INPROC_SERVER,
            )
        }
        .map_err(map_com)?;
        let count = unsafe { master.Count() }.map_err(map_com)?;
        for index in 0..count {
            let unique_id = unsafe { master.get_Item(index) }.map_err(map_com)?;
            let recorder: IDiscRecorder2 = unsafe {
                CoCreateInstance(
                    &MsftDiscRecorder2,
                    None::<&windows::core::IUnknown>,
                    CLSCTX_INPROC_SERVER,
                )
            }
            .map_err(map_com)?;
            if unsafe { recorder.InitializeDiscRecorder(&unique_id) }.is_err() {
                continue;
            }
            let Some(letter) = drive_letter(&recorder) else {
                continue;
            };
            if format!("{letter}:") == normalized {
                return Ok((recorder, normalized));
            }
        }
        Err(BurnError::InvalidRequest(format!(
            "la unidad {normalized} no es un dispositivo óptico detectado"
        )))
    }

    fn format_for(recorder: &IDiscRecorder2) -> Result<IDiscFormat2Data, BurnError> {
        let format: IDiscFormat2Data = unsafe {
            CoCreateInstance(
                &MsftDiscFormat2Data,
                None::<&windows::core::IUnknown>,
                CLSCTX_INPROC_SERVER,
            )
        }
        .map_err(map_com)?;
        unsafe { format.SetRecorder(recorder) }.map_err(map_com)?;
        let _ = unsafe { format.SetClientName(&BSTR::from("DVD Forge")) };
        Ok(format)
    }

    fn com_probe_media(device: &str) -> Result<MediaInfo, BurnError> {
        let (recorder, _) = find_recorder(device)?;
        let format = format_for(&recorder)?;
        let physical =
            unsafe { format.CurrentPhysicalMediaType() }.map_err(|_| BurnError::NoMedia)?;
        let kind = imapi_media_kind(physical.0);
        if kind == DiscMediaKind::Unknown {
            return Err(BurnError::NoMedia);
        }
        let state = unsafe { format.CurrentMediaStatus() }.map_err(map_com)?;
        let total_sectors = unsafe { format.TotalSectorsOnMedia() }.unwrap_or(0);
        let free_sectors = unsafe { format.FreeSectorsOnMedia() }.unwrap_or(0);
        let speed = unsafe { format.CurrentWriteSpeed() }
            .ok()
            .map(|sectors| sectors_per_second_to_speed_x_for(kind, sectors))
            .filter(|value| *value > 0.0);
        Ok(MediaInfo {
            kind,
            status: imapi_media_status(state.0, kind),
            capacity_bytes: imapi_sectors_to_bytes(total_sectors),
            free_bytes: imapi_sectors_to_bytes(free_sectors),
            writable: imapi_media_writable(state.0, kind),
            rewritable: kind.rewritable(),
            max_speed_x: speed,
        })
    }

    #[implement(IStream)]
    struct IsoStream {
        file: Mutex<std::fs::File>,
        total_bytes: u64,
        written: Arc<AtomicU64>,
        progress: ProgressSink,
        cancel: CancellationToken,
        started: Instant,
    }

    impl ISequentialStream_Impl for IsoStream_Impl {
        fn Read(&self, pv: *mut core::ffi::c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
            if self.cancel.is_cancelled() {
                return E_ABORT;
            }
            if pv.is_null() {
                return E_POINTER;
            }
            let mut file = match self.file.lock() {
                Ok(file) => file,
                Err(_) => return E_FAIL,
            };
            let buffer = unsafe { std::slice::from_raw_parts_mut(pv as *mut u8, cb as usize) };
            match file.read(buffer) {
                Ok(read) => {
                    if !pcbread.is_null() {
                        unsafe { *pcbread = read as u32 };
                    }
                    let written =
                        self.written.fetch_add(read as u64, Ordering::Relaxed) + read as u64;
                    let _ = self.progress.try_send(stream_progress(
                        written,
                        self.total_bytes,
                        self.started.elapsed(),
                    ));
                    HRESULT(0)
                }
                Err(_) => E_FAIL,
            }
        }

        fn Write(&self, _pv: *const core::ffi::c_void, _cb: u32, _pcbwritten: *mut u32) -> HRESULT {
            E_NOTIMPL
        }
    }

    impl IStream_Impl for IsoStream_Impl {
        fn Seek(
            &self,
            dlibmove: i64,
            dworigin: STREAM_SEEK,
            plibnewposition: *mut u64,
        ) -> WinResult<()> {
            let mut file = self
                .file
                .lock()
                .map_err(|_| windows::core::Error::from_hresult(E_FAIL))?;
            let position = if dworigin == STREAM_SEEK_SET {
                std::io::SeekFrom::Start(dlibmove as u64)
            } else if dworigin == STREAM_SEEK_CUR {
                std::io::SeekFrom::Current(dlibmove)
            } else if dworigin == STREAM_SEEK_END {
                std::io::SeekFrom::End(dlibmove)
            } else {
                return Err(windows::core::Error::from_hresult(E_NOTIMPL));
            };
            let new_position = file
                .seek(position)
                .map_err(|_| windows::core::Error::from_hresult(E_FAIL))?;
            if !plibnewposition.is_null() {
                unsafe { *plibnewposition = new_position };
            }
            Ok(())
        }

        fn SetSize(&self, _libnewsize: u64) -> WinResult<()> {
            Err(windows::core::Error::from_hresult(E_NOTIMPL))
        }

        fn CopyTo(
            &self,
            _pstm: Ref<IStream>,
            _cb: u64,
            _pcbread: *mut u64,
            _pcbwritten: *mut u64,
        ) -> WinResult<()> {
            Err(windows::core::Error::from_hresult(E_NOTIMPL))
        }

        fn Commit(&self, _grfcommitflags: &STGC) -> WinResult<()> {
            Ok(())
        }

        fn Revert(&self) -> WinResult<()> {
            Ok(())
        }

        fn LockRegion(&self, _liboffset: u64, _cb: u64, _dwlocktype: &LOCKTYPE) -> WinResult<()> {
            Err(windows::core::Error::from_hresult(E_NOTIMPL))
        }

        fn UnlockRegion(&self, _liboffset: u64, _cb: u64, _dwlocktype: u32) -> WinResult<()> {
            Err(windows::core::Error::from_hresult(E_NOTIMPL))
        }

        fn Stat(&self, pstatstg: *mut STATSTG, _grfstatflag: &STATFLAG) -> WinResult<()> {
            if pstatstg.is_null() {
                return Err(windows::core::Error::from_hresult(E_POINTER));
            }
            unsafe {
                (*pstatstg).cbSize = self.total_bytes;
                (*pstatstg).r#type = STGTY_STREAM.0 as u32;
                (*pstatstg).grfMode = STGM_READ;
            }
            Ok(())
        }

        fn Clone(&self) -> WinResult<IStream> {
            Err(windows::core::Error::from_hresult(E_NOTIMPL))
        }
    }

    fn burn_sync(
        request: BurnRequest,
        progress: ProgressSink,
        cancel: CancellationToken,
    ) -> Result<(BurnBackend, bool), BurnError> {
        if request.simulation {
            return Err(BurnError::InvalidRequest(
                "la simulación no está soportada por IMAPI2".to_string(),
            ));
        }
        let iso_size = std::fs::metadata(&request.iso_path).map_err(map_io)?.len();
        let media = com_probe_media(request.device.as_str())?;
        crate::validate_media_budget(&media, iso_size, request.speed_x)?;
        if !media.writable {
            return Err(BurnError::UnsupportedMedia(
                "IMAPI2 reporta el medio como no grabable".to_string(),
            ));
        }
        let (recorder, _) = find_recorder(request.device.as_str())?;
        let format = format_for(&recorder)?;
        if let Some(speed) = request.speed_x {
            let sectors = speed_x_to_sectors_per_second_for(media.kind, f32::from(speed));
            if sectors > 0 {
                unsafe { format.SetWriteSpeed(sectors, VARIANT_FALSE) }.map_err(map_com)?;
            }
        }
        if request.close_session {
            unsafe { format.SetForceMediaToBeClosed(VARIANT_TRUE) }.map_err(map_com)?;
        }
        let _ = unsafe { format.SetForceOverwrite(VARIANT_TRUE) };
        let file = std::fs::File::open(&request.iso_path).map_err(map_io)?;
        let stream: IStream = IsoStream {
            file: Mutex::new(file),
            total_bytes: iso_size,
            written: Arc::new(AtomicU64::new(0)),
            progress: progress.clone(),
            cancel: cancel.clone(),
            started: Instant::now(),
        }
        .into();
        if let Err(error) = unsafe { format.Write(&stream) } {
            if cancel.is_cancelled() {
                return Err(BurnError::Cancelled);
            }
            return Err(map_com(error));
        }
        let _ = progress.blocking_send(BurnProgress {
            written_bytes: iso_size,
            total_bytes: iso_size,
            percent: 100.0,
            speed_x: None,
            eta_secs: Some(0),
        });
        Ok((BurnBackend::Imapi2, false))
    }

    fn verify_sync(
        device: &DeviceId,
        iso: &std::path::Path,
        progress: &ProgressSink,
        cancel: &CancellationToken,
    ) -> Result<VerifyReport, BurnError> {
        let iso_size = std::fs::metadata(iso).map_err(map_io)?.len();
        if iso_size == 0 {
            return Err(BurnError::InvalidRequest("el ISO está vacío".to_string()));
        }
        let sectors_checked = iso_size.div_ceil(DVD_SECTOR_BYTES);
        // Hash the ISO on its own thread while the drive is read back.
        let iso_hasher = {
            let iso = iso.to_path_buf();
            std::thread::spawn(move || hash_file_sync(&iso))
        };
        let raw = raw_volume_path(device.as_str())?;
        let mut file = std::fs::File::open(&raw).map_err(map_io)?;
        use std::io::{Read, Seek, SeekFrom};
        file.seek(SeekFrom::Start(0)).map_err(map_io)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut remaining_iso = iso_size;
        let mut remaining_read = sectors_checked * DVD_SECTOR_BYTES;
        let mut read_total = 0u64;
        while remaining_read > 0 {
            if cancel.is_cancelled() {
                return Err(BurnError::Cancelled);
            }
            let chunk = remaining_read.min(buffer.len() as u64) as usize;
            let read = file.read(&mut buffer[..chunk]).map_err(map_io)?;
            if read == 0 {
                break;
            }
            if remaining_iso > 0 {
                let take = remaining_iso.min(read as u64) as usize;
                hasher.update(&buffer[..take]);
                remaining_iso -= take as u64;
            }
            remaining_read -= read as u64;
            read_total += read as u64;
            let _ = progress.try_send(BurnProgress {
                written_bytes: read_total.min(iso_size),
                total_bytes: iso_size,
                percent: (read_total.min(iso_size) as f32 / iso_size as f32) * 100.0,
                speed_x: None,
                eta_secs: None,
            });
        }
        if remaining_read > 0 || remaining_iso > 0 {
            return Err(BurnError::InvalidRequest(
                "la verificación IMAPI2 no cubrió el ISO".to_string(),
            ));
        }
        let iso_hash = iso_hasher.join().map_err(|_| {
            BurnError::Imapi("el hash del ISO terminó inesperadamente".to_string())
        })??;
        let disc_hash = hex::encode(hasher.finalize());
        Ok(VerifyReport {
            sectors_checked,
            mismatches: u64::from(iso_hash != disc_hash),
            sha256_iso: iso_hash,
            sha256_disc: disc_hash,
        })
    }

    fn hash_file_sync(path: &std::path::Path) -> Result<String, BurnError> {
        use std::io::Read;
        let mut file = std::fs::File::open(path).map_err(map_io)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(map_io)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hex::encode(hasher.finalize()))
    }

    fn eject_sync(device: &DeviceId) -> Result<(), BurnError> {
        let (recorder, _) = find_recorder(device.as_str())?;
        unsafe { recorder.EjectMedia() }.map_err(map_com)
    }

    fn map_io(error: std::io::Error) -> BurnError {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            BurnError::PermissionDenied(error.to_string())
        } else {
            BurnError::Io(error)
        }
    }

    #[async_trait::async_trait]
    impl DiscBurner for WindowsBurner {
        fn backend(&self) -> BurnBackend {
            BurnBackend::Imapi2
        }

        async fn list_devices(&self) -> Result<Vec<OpticalDevice>, BurnError> {
            tokio::task::spawn_blocking(com_list_devices)
                .await
                .map_err(map_join)?
        }

        async fn probe_media(&self, device: &DeviceId) -> Result<MediaInfo, BurnError> {
            let device = device.clone();
            tokio::task::spawn_blocking(move || {
                let _com = ComApartment::new()?;
                com_probe_media(device.as_str())
            })
            .await
            .map_err(map_join)?
        }

        async fn burn(
            &self,
            request: BurnRequest,
            progress: ProgressSink,
            cancel: CancellationToken,
        ) -> Result<BurnReport, BurnError> {
            let started = Instant::now();
            let iso_size = std::fs::metadata(&request.iso_path).map_err(map_io)?.len();
            let progress_for_verify = progress.clone();
            let cancel_for_burn = cancel.clone();
            let verify = request.verify;
            let eject = request.eject;
            let device_for_verify = request.device.clone();
            let device_for_eject = request.device.clone();
            let iso_path = request.iso_path.clone();
            let result = tokio::task::spawn_blocking(move || {
                let _com = ComApartment::new()?;
                burn_sync(request, progress, cancel_for_burn)
            })
            .await
            .map_err(map_join)?;
            let (backend, mut verified) = result?;
            if verify {
                let cancel_for_verify = cancel.clone();
                let report = tokio::task::spawn_blocking(move || {
                    let _com = ComApartment::new()?;
                    verify_sync(
                        &device_for_verify,
                        &iso_path,
                        &progress_for_verify,
                        &cancel_for_verify,
                    )
                })
                .await
                .map_err(map_join)??;
                if report.mismatches > 0 {
                    return Err(BurnError::VerifyFailed {
                        iso: report.sha256_iso,
                        disc: report.sha256_disc,
                    });
                }
                verified = true;
            }
            if eject {
                tokio::task::spawn_blocking(move || {
                    let _com = ComApartment::new()?;
                    eject_sync(&device_for_eject)
                })
                .await
                .map_err(map_join)??;
            }
            let elapsed = started.elapsed();
            let average_speed_x = if elapsed.as_secs_f64() > 0.0 {
                Some(((iso_size as f64 / elapsed.as_secs_f64()) / DVD_1X_BYTES_PER_SECOND) as f32)
            } else {
                None
            };
            Ok(BurnReport {
                backend,
                elapsed_ms: elapsed.as_millis() as u64,
                average_speed_x,
                verified,
            })
        }

        async fn verify(
            &self,
            device: &DeviceId,
            iso: &std::path::Path,
            progress: ProgressSink,
        ) -> Result<VerifyReport, BurnError> {
            let device = device.clone();
            let iso = iso.to_path_buf();
            tokio::task::spawn_blocking(move || {
                let _com = ComApartment::new()?;
                verify_sync(&device, &iso, &progress, &CancellationToken::new())
            })
            .await
            .map_err(map_join)?
        }

        async fn eject(&self, device: &DeviceId) -> Result<(), BurnError> {
            let device = device.clone();
            tokio::task::spawn_blocking(move || {
                let _com = ComApartment::new()?;
                eject_sync(&device)
            })
            .await
            .map_err(map_join)?
        }
    }
}

#[cfg(target_os = "windows")]
pub use platform::WindowsBurner;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_dvd_speeds() {
        let sectors = speed_x_to_sectors_per_second(1.0);
        assert_eq!(sectors, 676);
        assert!((sectors_per_second_to_speed_x(sectors) - 1.0).abs() < 0.01);
        assert_eq!(speed_x_to_sectors_per_second(0.0), 0);
        assert_eq!(sectors_per_second_to_speed_x(-1), 0.0);
    }

    #[test]
    fn maps_imapi_media_types_and_states() {
        assert_eq!(imapi_media_kind(4), DiscMediaKind::DvdRom);
        assert_eq!(imapi_media_kind(6), DiscMediaKind::DvdPlusR);
        assert_eq!(imapi_media_kind(7), DiscMediaKind::DvdPlusRw);
        assert_eq!(imapi_media_kind(9), DiscMediaKind::DvdR);
        assert_eq!(imapi_media_kind(10), DiscMediaKind::DvdRw);
        assert_eq!(imapi_media_kind(12), DiscMediaKind::Unknown);
        assert_eq!(imapi_media_kind(18), DiscMediaKind::BdR);
        assert_eq!(imapi_media_kind(19), DiscMediaKind::BdRe);
        assert_eq!(
            speed_x_to_sectors_per_second_for(DiscMediaKind::BdR, 1.0),
            2_195
        );
        assert_eq!(imapi_media_kind(99), DiscMediaKind::Unknown);
        assert_eq!(
            imapi_media_status(IMAPI_BLANK_STATE, DiscMediaKind::DvdR),
            MediaStatus::Blank
        );
        assert_eq!(
            imapi_media_status(IMAPI_APPENDABLE_STATE, DiscMediaKind::DvdR),
            MediaStatus::Appendable
        );
        assert_eq!(
            imapi_media_status(IMAPI_FINALIZED_STATE, DiscMediaKind::DvdRw),
            MediaStatus::Closed
        );
        assert!(imapi_media_writable(
            IMAPI_BLANK_STATE,
            DiscMediaKind::DvdPlusR
        ));
        assert!(!imapi_media_writable(
            IMAPI_WRITE_PROTECTED_STATE,
            DiscMediaKind::DvdPlusR
        ));
    }

    #[test]
    fn normalizes_windows_volume_paths() {
        assert_eq!(raw_volume_path("D:").unwrap(), "\\\\.\\D:");
        assert_eq!(raw_volume_path("d").unwrap(), "\\\\.\\D:");
        assert_eq!(normalize_drive_id("D:").unwrap(), "D:");
        assert_eq!(normalize_drive_id("d").unwrap(), "D:");
        assert!(raw_volume_path("disk2").is_err());
    }

    #[test]
    fn tracks_stream_progress() {
        let progress = stream_progress(500, 1000, Duration::from_secs(1));
        assert_eq!(progress.written_bytes, 500);
        assert_eq!(progress.total_bytes, 1000);
        assert!((progress.percent - 50.0).abs() < 0.01);
        assert!(progress.speed_x.is_some());
        assert!(progress.eta_secs.is_some());

        let mut tracker = StreamProgressTracker::new(1000);
        let first = tracker.record(250);
        assert_eq!(first.written_bytes, 250);
        assert_eq!(tracker.written_bytes(), 250);
        let second = tracker.record(750);
        assert_eq!(second.written_bytes, 1000);
        assert!((second.percent - 100.0).abs() < 0.01);
    }

    #[test]
    fn converts_sectors_to_bytes() {
        assert_eq!(imapi_sectors_to_bytes(10), 20_480);
        assert_eq!(imapi_sectors_to_bytes(-1), 0);
    }
}
