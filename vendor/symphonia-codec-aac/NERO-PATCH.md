# Strict AAC-LC decoding

This directory contains Symphonia's `symphonia-codec-aac` 0.6.1 source under MPL-2.0.
Original crate archive: https://crates.io/api/v1/crates/symphonia-codec-aac/0.6.1/download
Original archive SHA-256: f5bf8e39552d34a3c4c98333370e62f48c92456d2e814f273b5c3ad7c4a5f45c

The original source detects in-band SBR but returns the incomplete AAC-LC core.
The only source change rejects that packet before PCM is returned. This also
rejects backwards-compatible HE-AAC in ADTS where no AudioSpecificConfig exists.
It adds no SBR decoder, changes no AAC-LC signal processing, and never skips errors.
Regression coverage uses real AAC-LC and HE-AAC M4A/ADTS encodings in the desktop
codec matrix. The upstream `LICENSE` and source headers are preserved.
