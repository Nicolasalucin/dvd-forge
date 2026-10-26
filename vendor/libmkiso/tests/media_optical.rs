use libmkiso::boot_media::{
    Error,
    optical::{self, BootPolicy, CreateOptions, OperationContext, OperationLimits},
    progress::{CancellationToken, NoProgress, Phase, ProgressState},
};
use std::{fs, process::Command};

#[test]
fn creation_is_reproducible_and_roundtrips_independently() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::create_dir(source.join("nested")).unwrap();
    fs::write(source.join("nested/日本語.txt"), b"hello world").unwrap();
    let options = CreateOptions {
        iso: libmkiso::IsoOptions {
            joliet: true,
            filename_policy: libmkiso::FilenamePolicy::Mangle,
            ..Default::default()
        },
        ..Default::default()
    };
    let token = CancellationToken::default();
    let mut observer = NoProgress;
    let mut ctx = OperationContext {
        observer: &mut observer,
        cancellation: &token,
        operation_id: 1,
    };
    let first = temp.path().join("first.iso");
    let second = temp.path().join("second.iso");
    let report = optical::create(&source, &first, &options, &mut ctx).unwrap();
    optical::create(&source, &second, &options, &mut ctx).unwrap();
    assert_eq!(fs::read(&first).unwrap(), fs::read(&second).unwrap());
    assert!(!report.bootable);
    let output = temp.path().join("extracted");
    optical::extract(&first, &output, &OperationLimits::default(), &mut ctx).unwrap();
    assert_eq!(
        fs::read(output.join("nested/日本語.txt")).unwrap(),
        b"hello world"
    );
    let seven = ["7z", "7zz"]
        .into_iter()
        .find(|name| Command::new(name).arg("i").output().is_ok());
    if let Some(seven) = seven {
        let external = temp.path().join("external");
        let result = Command::new(seven)
            .arg("x")
            .arg(&first)
            .arg(format!("-o{}", external.display()))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert_eq!(
            fs::read(external.join("nested/日本語.txt")).unwrap(),
            b"hello world"
        );
    } else {
        eprintln!("independent reader unavailable");
    }
}
#[test]
fn source_mutation_never_publishes_output() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let payload = source.join("DATA.TXT");
    fs::write(&payload, b"first").unwrap();
    let output = temp.path().join("out.iso");
    let token = CancellationToken::default();
    let mut observer = |event: &libmkiso::boot_media::progress::ProgressEvent| {
        if event.phase == Phase::EmitImage && event.state == ProgressState::Started {
            fs::write(&payload, b"other").unwrap();
        }
    };
    let mut ctx = OperationContext {
        observer: &mut observer,
        cancellation: &token,
        operation_id: 2,
    };
    let error = optical::create(&source, &output, &CreateOptions::default(), &mut ctx)
        .expect_err("mutated source must not be published");
    // The writer may detect drift before the orchestration snapshot check.
    assert!(
        matches!(error, Error::InvalidInput(_))
            || matches!(&error, Error::Io(source) if source.to_string() == "host identity, generation or size changed")
            || matches!(&error, Error::Unsupported(message) if message == "source file size changed during creation"),
        "unexpected mutation error: {error}"
    );
    assert!(!output.exists());
}
#[test]
fn cancellation_during_emission_keeps_old_output_and_cleans_staging() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("DATA.TXT"), b"payload").unwrap();
    let output = temp.path().join("out.iso");
    fs::write(&output, b"original").unwrap();
    let token = CancellationToken::default();
    let mut events = Vec::new();
    let mut observer = |event: &libmkiso::boot_media::progress::ProgressEvent| {
        events.push(event.clone());
        if event.phase == Phase::EmitImage && event.state == ProgressState::Started {
            token.cancel();
        }
    };
    let mut ctx = OperationContext {
        observer: &mut observer,
        cancellation: &token,
        operation_id: 3,
    };
    let options = CreateOptions {
        replace: true,
        ..Default::default()
    };
    assert!(matches!(
        optical::create(&source, &output, &options, &mut ctx),
        Err(Error::Cancelled)
    ));
    let _ = ctx;
    let _ = observer;
    assert_eq!(fs::read(&output).unwrap(), b"original");
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 2);
    assert!(events.iter().any(|e| e.state == ProgressState::Cancelled));
}
#[cfg(unix)]
#[test]
fn source_hard_link_output_is_protected_even_with_replace() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let payload = source.join("DATA.TXT");
    fs::write(&payload, b"payload").unwrap();
    let output = temp.path().join("out.iso");
    fs::hard_link(&payload, &output).unwrap();
    let token = CancellationToken::default();
    let mut observer = NoProgress;
    let mut ctx = OperationContext {
        observer: &mut observer,
        cancellation: &token,
        operation_id: 4,
    };
    let options = CreateOptions {
        replace: true,
        ..Default::default()
    };
    assert!(matches!(
        optical::create(&source, &output, &options, &mut ctx),
        Err(Error::InvalidInput(_))
    ));
    assert_eq!(fs::read(payload).unwrap(), b"payload");
}
#[test]
fn payload_budget_rejects_extraction_before_creating_directory() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("DATA.TXT"), b"payload").unwrap();
    let image = temp.path().join("out.iso");
    let token = CancellationToken::default();
    let mut observer = NoProgress;
    let mut ctx = OperationContext {
        observer: &mut observer,
        cancellation: &token,
        operation_id: 5,
    };
    optical::create(&source, &image, &CreateOptions::default(), &mut ctx).unwrap();
    let output = temp.path().join("extract");
    let limits = OperationLimits {
        max_output_bytes: 2,
        ..Default::default()
    };
    assert!(matches!(
        optical::extract(&image, &output, &limits, &mut ctx),
        Err(Error::Resource(_))
    ));
    assert!(!output.exists());
}
#[test]
fn plain_data_repack_changes_selected_payload_and_retains_source() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let overlay = temp.path().join("overlay");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&overlay).unwrap();
    fs::write(source.join("DATA.TXT"), b"first").unwrap();
    fs::write(source.join("KEEP.TXT"), b"keep").unwrap();
    fs::write(overlay.join("DATA.TXT"), b"new").unwrap();
    let image = temp.path().join("source.iso");
    let output = temp.path().join("new.iso");
    let token = CancellationToken::default();
    let mut observer = NoProgress;
    let mut ctx = OperationContext {
        observer: &mut observer,
        cancellation: &token,
        operation_id: 6,
    };
    optical::create(&source, &image, &CreateOptions::default(), &mut ctx).unwrap();
    let original = fs::read(&image).unwrap();
    optical::repack(
        &image,
        &overlay,
        &output,
        &CreateOptions::default(),
        BootPolicy::Remove,
        &mut ctx,
    )
    .unwrap();
    assert_eq!(fs::read(&image).unwrap(), original);
    let extracted = temp.path().join("extracted");
    optical::extract(&output, &extracted, &OperationLimits::default(), &mut ctx).unwrap();
    assert_eq!(fs::read(extracted.join("DATA.TXT")).unwrap(), b"new");
    assert_eq!(fs::read(extracted.join("KEEP.TXT")).unwrap(), b"keep");
}

#[test]
fn progress_records_real_bytes_without_changing_output() {
    use libmkiso::boot_media::progress::{ProgressEvent, ProgressUnit};
    use std::collections::HashMap;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("DATA.TXT"), vec![7; 150_000]).unwrap();
    let token = CancellationToken::default();
    let mut events = Vec::new();
    {
        let mut observer = |event: &ProgressEvent| events.push(event.clone());
        let mut ctx = OperationContext {
            observer: &mut observer,
            cancellation: &token,
            operation_id: 100,
        };
        optical::create(
            &source,
            &temp.path().join("with.iso"),
            &CreateOptions::default(),
            &mut ctx,
        )
        .unwrap();
    }
    let mut sink = NoProgress;
    let mut ctx = OperationContext {
        observer: &mut sink,
        cancellation: &token,
        operation_id: 100,
    };
    optical::create(
        &source,
        &temp.path().join("without.iso"),
        &CreateOptions::default(),
        &mut ctx,
    )
    .unwrap();
    assert_eq!(
        fs::read(temp.path().join("with.iso")).unwrap(),
        fs::read(temp.path().join("without.iso")).unwrap()
    );
    let mut previous = HashMap::new();
    for event in &events {
        let key = (event.operation_id, format!("{:?}", event.phase));
        let (completed, terminal) = previous.entry(key).or_insert((0, false));
        assert!(!*terminal, "event after terminal: {event:?}");
        assert!(
            event.completed >= *completed,
            "counter decreased: {event:?}"
        );
        if let Some(total) = event.total {
            assert!(event.completed <= total);
            if event.state == ProgressState::Finished {
                assert_eq!(event.completed, total);
            }
        }
        *completed = event.completed;
        *terminal = matches!(
            event.state,
            ProgressState::Finished | ProgressState::Failed | ProgressState::Cancelled
        );
    }
    assert!(previous.values().all(|(_, terminal)| *terminal));
    let emitted = events
        .iter()
        .find(|e| e.phase == Phase::EmitImage && e.state == ProgressState::Finished)
        .unwrap();
    assert_eq!(emitted.unit, ProgressUnit::Bytes);
    assert_eq!(
        emitted.completed,
        fs::metadata(temp.path().join("with.iso")).unwrap().len()
    );
    assert!(
        events
            .iter()
            .any(|e| e.phase == Phase::HashInputs && e.completed == 150_000)
    );
}
#[test]
fn empty_directory_mutation_rejects_publication() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    fs::write(source.join("DATA.TXT"), b"payload").unwrap();
    let token = CancellationToken::default();
    let mut observer = |event: &libmkiso::boot_media::progress::ProgressEvent| {
        if event.phase == Phase::EmitImage && event.state == ProgressState::Started {
            fs::create_dir(source.join("ADDED")).unwrap();
        }
    };
    let mut ctx = OperationContext {
        observer: &mut observer,
        cancellation: &token,
        operation_id: 200,
    };
    let output = temp.path().join("out.iso");
    assert!(matches!(
        optical::create(&source, &output, &CreateOptions::default(), &mut ctx),
        Err(Error::InvalidInput(_))
    ));
    assert!(!output.exists());
}

#[test]
fn malformed_image_finishes_index_phase_with_failure() {
    let temp = tempfile::tempdir().unwrap();
    let image = temp.path().join("bad.iso");
    fs::write(&image, b"truncated").unwrap();
    let token = CancellationToken::default();
    let mut events = Vec::new();
    {
        let mut observer =
            |event: &libmkiso::boot_media::progress::ProgressEvent| events.push(event.clone());
        let mut ctx = OperationContext {
            observer: &mut observer,
            cancellation: &token,
            operation_id: 300,
        };
        assert!(optical::inspect(&image, &OperationLimits::default(), &mut ctx).is_err());
    }
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].phase, Phase::OpenImage);
    assert_eq!(events[0].state, ProgressState::Started);
    assert_eq!(events[1].state, ProgressState::Failed);
}
