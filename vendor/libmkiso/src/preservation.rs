//! Inspection and preservation capabilities. Content copying does not imply
//! faithful capture. Unknown fields and native opaque bytes require preflight.

/// Inspection state; absence is different from a field that was never inspected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Field<T> {
    Absent,
    Uninspected,
    Present(T),
    /// Understood on input, but unavailable in the destination writer.
    NotWritable(T),
}

/// A native timestamp retains encoding and precision instead of guessing UTC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeTimestamp {
    pub encoding: TimestampEncoding,
    pub bytes: Vec<u8>,
    pub precision: TimestampPrecision,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampEncoding {
    IsoShort,
    IsoLong,
    Udf,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampPrecision {
    Seconds,
    Hundredths,
    Microseconds,
}

/// Native metadata is source scoped. Opaque fields are never automatically
/// safe to transplant to a different object or image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    pub native_name: Field<Vec<u8>>,
    pub timestamps: Field<Vec<(u8, NativeTimestamp)>>,
    pub ownership: Field<(u32, u32)>,
    pub permissions: Field<u32>,
    pub extensions: Field<Vec<u8>>,
    pub opaque: Field<Vec<u8>>,
}
impl Default for Metadata {
    fn default() -> Self {
        Self {
            native_name: Field::Uninspected,
            timestamps: Field::Uninspected,
            ownership: Field::Uninspected,
            permissions: Field::Uninspected,
            extensions: Field::Uninspected,
            opaque: Field::Uninspected,
        }
    }
}
impl Metadata {
    /// Declared storage for this metadata value, including retained native
    /// bytes and timestamp descriptors. Saturation makes overflow exceed any
    /// finite inventory budget rather than wrapping to a small allocation.
    pub fn allocation_bytes(&self) -> usize {
        fn bytes(value: &Field<Vec<u8>>) -> usize {
            match value {
                Field::Present(value) | Field::NotWritable(value) => value.len(),
                _ => 0,
            }
        }
        let timestamps = match &self.timestamps {
            Field::Present(values) | Field::NotWritable(values) => values.iter().fold(
                values
                    .len()
                    .saturating_mul(std::mem::size_of::<(u8, NativeTimestamp)>()),
                |sum, (_, stamp)| sum.saturating_add(stamp.bytes.len()),
            ),
            _ => 0,
        };
        [
            bytes(&self.native_name),
            timestamps,
            bytes(&self.extensions),
            bytes(&self.opaque),
        ]
        .into_iter()
        .fold(std::mem::size_of::<Self>(), usize::saturating_add)
    }
    /// Baseline ISO entry inspection. Fields outside the selected namespace
    /// remain uninspected; lack of a PX/TF record is not inferred from another
    /// namespace. Root metadata must be inspected separately.
    pub fn from_iso(entry: &crate::IsoEntry) -> Self {
        let mut metadata = Self {
            native_name: Field::Present(entry.raw_name.clone()),
            ..Self::default()
        };
        if let Some(unix) = &entry.unix {
            metadata.ownership = Field::Present((unix.uid, unix.gid));
            metadata.permissions = Field::Present(unix.mode);
            metadata.timestamps = Field::Present(
                unix.timestamps
                    .iter()
                    .map(|(kind, bytes)| {
                        let long = bytes.len() == 17;
                        (
                            *kind,
                            NativeTimestamp {
                                encoding: if long {
                                    TimestampEncoding::IsoLong
                                } else {
                                    TimestampEncoding::IsoShort
                                },
                                bytes: bytes.clone(),
                                precision: if long {
                                    TimestampPrecision::Hundredths
                                } else {
                                    TimestampPrecision::Seconds
                                },
                            },
                        )
                    })
                    .collect(),
            );
        }
        metadata
    }
    /// Baseline UDF names are inspected, but the compatibility entry does not
    /// expose FE timestamps, ownership, permissions or implementation-use data.
    pub fn from_udf(entry: &crate::UdfEntry) -> Self {
        Self {
            native_name: Field::Present(entry.raw_name.clone()),
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionProfile {
    Physical,
    Virtual,
    Sparable,
    Metadata,
    MetadataSparable,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    IsoPrimary,
    IsoJoliet,
    IsoRockRidge,
    Udf {
        revision: u16,
        partition: PartitionProfile,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    Supported,
    Uninspected,
    Gated,
    Unsupported,
}
/// Separate inventory and emission capabilities, not a claim of faithful capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub content: Support,
    pub native_names: Support,
    pub timestamps: Support,
    pub ownership: Support,
    pub permissions: Support,
    pub hard_links: Support,
    pub streams: Support,
    pub sparse: Support,
    pub opaque: Support,
}
impl Profile {
    fn known(self) -> bool {
        match self {
            Self::Udf {
                revision,
                partition,
            } => {
                matches!(revision, 0x102 | 0x150 | 0x200 | 0x201 | 0x250 | 0x260)
                    && (!matches!(
                        partition,
                        PartitionProfile::Metadata | PartitionProfile::MetadataSparable
                    ) || revision >= 0x250)
            }
            _ => true,
        }
    }
    pub fn read_capabilities(self) -> Capabilities {
        let mut c = Capabilities {
            content: Support::Supported,
            native_names: Support::Supported,
            timestamps: Support::Supported,
            ownership: Support::Uninspected,
            permissions: Support::Uninspected,
            hard_links: Support::Unsupported,
            streams: Support::Unsupported,
            sparse: Support::Unsupported,
            opaque: Support::Uninspected,
        };
        match self {
            Self::IsoRockRidge => {
                c.timestamps = Support::Supported;
                c.ownership = Support::Supported;
                c.permissions = Support::Supported;
                c.hard_links = Support::Supported;
            }
            Self::Udf { .. } => {
                c.timestamps = Support::Supported;
                c.ownership = Support::Supported;
                c.permissions = Support::Supported;
                c.hard_links = Support::Supported;
                c.streams = Support::Supported;
                c.sparse = Support::Supported;
            }
            _ => {}
        }
        if !self.known() {
            c.content = Support::Unsupported;
            c.native_names = Support::Unsupported;
        }
        c
    }
    pub fn write_capabilities(self) -> Capabilities {
        let mut c = self.read_capabilities();
        if matches!(self, Self::Udf { revision, partition: PartitionProfile::Virtual | PartitionProfile::Sparable }
            if !(0x150..=0x201).contains(&revision))
        {
            c.content = Support::Unsupported;
        }
        // Existing writers generate names and timestamps; exact native fields
        // are not accepted by the authoring API. Gate preservation accordingly.
        c.native_names = Support::Gated;
        c.timestamps = Support::Gated;
        c.ownership = Support::Gated;
        c.permissions = Support::Gated;
        c.opaque = Support::Unsupported;
        if matches!(self, Self::Udf { revision, .. } if revision < 0x200) {
            c.streams = Support::Unsupported;
        }
        if matches!(self, Self::IsoRockRidge) {
            c.hard_links = Support::Gated;
        }
        c
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Explicitly permits metadata transformation; the report lists lost fields.
    ContentOnly,
    /// Every inspected field, root metadata and topology must be preserved.
    Faithful,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// None denotes the root, otherwise the source inventory entry index.
    pub entry: Option<usize>,
    pub field: &'static str,
    pub reason: &'static str,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub policy: Policy,
    pub issues: Vec<Issue>,
}
impl Report {
    pub fn allowed(&self) -> bool {
        !self.issues.iter().any(|issue| issue.field == "profile")
            && (self.policy == Policy::ContentOnly || self.issues.is_empty())
    }
    pub fn faithful(&self) -> bool {
        self.policy == Policy::Faithful && self.issues.is_empty()
    }
}

/// Preflight root and entry fields against an explicitly selected destination.
/// Name/topology validation remains mandatory in the writer's inventory pass.
/// Advanced faithful capture is gated until native metadata emission exists.
pub fn preflight(
    profile: Profile,
    policy: Policy,
    root: &Metadata,
    entries: &[Metadata],
) -> Report {
    preflight_iter(profile, policy, root, entries)
}

/// Borrowed inventory variant that avoids copying native metadata for preflight.
/// Issue ordinals follow iterator order, including any separately indexed streams.
pub fn preflight_iter<'a>(
    profile: Profile,
    policy: Policy,
    root: &Metadata,
    entries: impl IntoIterator<Item = &'a Metadata>,
) -> Report {
    let capabilities = profile.write_capabilities();
    let mut report = Report {
        policy,
        issues: Vec::new(),
    };
    if capabilities.content != Support::Supported {
        report.issues.push(Issue {
            entry: None,
            field: "profile",
            reason: "unsupported destination profile",
        });
    }
    if policy == Policy::Faithful {
        report.issues.push(Issue {
            entry: None,
            field: "preservation_profile",
            reason: "faithful native metadata emission is gated",
        });
    }
    fn field<T>(
        report: &mut Report,
        entry: Option<usize>,
        name: &'static str,
        value: &Field<T>,
        support: Support,
    ) {
        let reason = match value {
            Field::Absent => return,
            Field::Uninspected => "field has not been inspected",
            Field::NotWritable(_) => "input field is understood but not writable",
            Field::Present(_) if support == Support::Supported => return,
            Field::Present(_) => "native preservation is unavailable for destination",
        };
        report.issues.push(Issue {
            entry,
            field: name,
            reason,
        });
    }
    for (entry, metadata) in std::iter::once((None, root))
        .chain(entries.into_iter().enumerate().map(|(i, m)| (Some(i), m)))
    {
        field(
            &mut report,
            entry,
            "native_name",
            &metadata.native_name,
            capabilities.native_names,
        );
        field(
            &mut report,
            entry,
            "timestamps",
            &metadata.timestamps,
            capabilities.timestamps,
        );
        field(
            &mut report,
            entry,
            "ownership",
            &metadata.ownership,
            capabilities.ownership,
        );
        field(
            &mut report,
            entry,
            "permissions",
            &metadata.permissions,
            capabilities.permissions,
        );
        field(
            &mut report,
            entry,
            "extensions",
            &metadata.extensions,
            Support::Gated,
        );
        field(
            &mut report,
            entry,
            "opaque",
            &metadata.opaque,
            capabilities.opaque,
        );
    }
    report
}

/// Inspect ECMA-167 File Entry or Extended File Entry metadata after the reader
/// has validated its tag and bounds. Native timestamps retain all twelve bytes.
/// Extended attributes are understood as an opaque, unwritable region; native
/// implementation-use fields still require inspection and relocation policy.
pub(crate) fn inspect_udf_file_entry(descriptor: &[u8], raw_name: Vec<u8>) -> Metadata {
    let extended = descriptor.get(..2) == Some(&266u16.to_le_bytes());
    let header = if extended { 216 } else { 176 };
    let word = |offset: usize| {
        descriptor
            .get(offset..offset + 4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
    };
    let timestamps: Vec<_> = if extended {
        vec![(4, 80), (2, 92), (1, 104), (8, 116)]
    } else {
        vec![(4, 72), (2, 84), (8, 96)]
    };
    let timestamps = timestamps
        .into_iter()
        .map(|(kind, offset)| {
            descriptor.get(offset..offset + 12).map(|bytes| {
                (
                    kind,
                    NativeTimestamp {
                        encoding: TimestampEncoding::Udf,
                        bytes: bytes.to_vec(),
                        precision: TimestampPrecision::Microseconds,
                    },
                )
            })
        })
        .collect::<Option<Vec<_>>>();
    let extensions =
        word(header - 8).and_then(|length| descriptor.get(header..header + length as usize));
    Metadata {
        native_name: Field::Present(raw_name),
        timestamps: timestamps.map_or(Field::Uninspected, Field::Present),
        ownership: match (word(36), word(40)) {
            (Some(uid), Some(gid)) => Field::Present((uid, gid)),
            _ => Field::Uninspected,
        },
        permissions: word(44).map_or(Field::Uninspected, Field::Present),
        extensions: match extensions {
            Some([]) => Field::Absent,
            Some(bytes) => Field::NotWritable(bytes.to_vec()),
            None => Field::Uninspected,
        },
        opaque: Field::Uninspected,
    }
}
