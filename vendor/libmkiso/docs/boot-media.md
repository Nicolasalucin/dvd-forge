# boot-media

Media orchestration APIs for the `mkiso` executable. The optical filesystem
implementation remains in `libmkiso`; the cli-gated boot_media module adds strict version-one
TOML manifests, bounded deterministic inventories, SHA-256 input fingerprints,
read-only plans, source guards, progress/cancellation events and device policy.

`manifest::read_manifest` and `manifest::read_multiboot` reject unknown fields.
`plan::plan_manifest` resolves source/output against the manifest directory and
boot/payload paths against the source tree. `plan::plan_multiboot` hashes each ISO
separately and records `untested` entry status. A plan's `unsupported` list must
be empty before building. A missing exact image size is serialized as `null`,
never presented as a payload-only capacity estimate.

`optical` provides data ISO creation, bounded inspection/extraction, payload
verification and restricted repacking. Output publication uses a temporary
sibling, synchronization and source checks. The events and cancellation token
remain usable without any terminal dependency. Linux `device` APIs require an
explicit device, destructive authorization and matching inspected identity.

See [the capability inventory](../../docs/mkiso-capabilities.md) for limitations
and evidence. Structural checks do not establish firmware or installer support.
