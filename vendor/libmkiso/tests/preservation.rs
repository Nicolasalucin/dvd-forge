use libmkiso::preservation::{
    Field, Metadata, PartitionProfile, Policy, Profile, Support, preflight,
};

#[test]
fn unknown_root_and_entry_metadata_cannot_pass_faithful_preflight() {
    let report = preflight(
        Profile::IsoRockRidge,
        Policy::Faithful,
        &Metadata::default(),
        &[Metadata::default()],
    );
    assert!(!report.allowed());
    assert!(!report.faithful());
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.entry.is_none() && issue.field == "timestamps")
    );
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.entry == Some(0) && issue.field == "ownership")
    );
}

#[test]
fn explicit_content_transformation_reports_losses() {
    let metadata = Metadata {
        native_name: Field::Present(vec![8, b'a']),
        opaque: Field::Present(vec![1, 2]),
        ..Metadata::default()
    };
    let profile = Profile::Udf {
        revision: 0x260,
        partition: PartitionProfile::Metadata,
    };
    let report = preflight(profile, Policy::ContentOnly, &metadata, &[]);
    assert!(report.allowed());
    assert!(!report.faithful());
    assert!(report.issues.iter().any(|issue| issue.field == "opaque"));
    assert_eq!(profile.read_capabilities().streams, Support::Supported);
    assert_eq!(profile.write_capabilities().opaque, Support::Unsupported);
}

#[test]
fn understood_unwritable_is_distinct_from_absent() {
    let mut root = Metadata {
        native_name: Field::Absent,
        timestamps: Field::Absent,
        ownership: Field::Absent,
        permissions: Field::Absent,
        extensions: Field::Absent,
        opaque: Field::Absent,
    };
    root.ownership = Field::NotWritable((1000, 100));
    let report = preflight(Profile::IsoPrimary, Policy::Faithful, &root, &[]);
    assert!(report.issues.iter().any(|issue| issue.field == "ownership"
        && issue.reason == "input field is understood but not writable"));
}

#[test]
fn invalid_udf_revision_partition_is_not_advertised() {
    for profile in [
        Profile::Udf {
            revision: 0x300,
            partition: PartitionProfile::Physical,
        },
        Profile::Udf {
            revision: 0x102,
            partition: PartitionProfile::Metadata,
        },
    ] {
        assert_eq!(profile.read_capabilities().content, Support::Unsupported);
        assert!(!preflight(profile, Policy::Faithful, &Metadata::default(), &[]).allowed());
    }
}
