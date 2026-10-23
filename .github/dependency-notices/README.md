# Pinned development-client legal inputs

`manifest.json` binds every supplemental legal file to reviewed package metadata,
the crates.io archive checksum, and the exact published Git revision/path.
Inputs are committed unchanged or explicitly marked as downstream provenance.
The collector checks hashes and identities locally; it does not download notices.
Unreviewed missing notices, changed sources, and unknown licenses still fail.
Every external package must match one checksum-bearing crates.io lock entry;
private registries, Git sources, and unlocked packages are never copied. Any new
version of a reviewed package name requires renewed notice review, even when it
contains a root license file. Only reviewed text-notice formats are copied.

Most inputs restore original repository-root legal files omitted from published
subcrates. The constgebra 0.1.4 entry instead selects its explicitly declared
Apache-2.0 option: canonical full license text is clearly identified as canonical,
not an upstream file. Its original dual-license declaration is retained, and no
absent copyright owner, year, or upstream NOTICE is invented.

Floating-point dependency supplements preserve legally relevant source headers
and inherited upstream license text without bundling implementation source.
Every excerpt has exact source provenance and content hashes. Existing packaged
licenses remain included separately. Updating these dependencies requires review
of those source notices, not copying old supplements onto a newer version.

`cargo-about --locked --fail` remains an independent recognized-license gate.
These inputs are not a legal-clearance certification or a general missing-license
exception. The artifact contains engine code and legal notices, never game data.
