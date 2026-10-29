# Retired Swift application

This directory preserves the previous macOS implementation and its Swift tests. It is not the Nero desktop application and is not an installation target. Do not launch or install the archived application during Nero validation.

Nero uses `apps/desktop`, `crates/nero-core` and `crates/nero-cli`. Its macOS audio converter lives in `scripts/audio-prepare`; the small historical MOSS metadata lives in `crates/nero-core/resources/models`. Model weights and user recordings are not archived here.

The historical package and source retain their original module names. Optional legacy builds use `.work/legacy-swift` at the repository root; they are not part of the current desktop release path. This archive can be removed after the replacement is complete.
