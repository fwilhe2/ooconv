// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The caps every package reader holds a stranger's zip to, both families alike. Lifted from grind
//! (`core/src/odf/package.rs`).

use std::io::Read;

/// No document has this many parts. A package that does is a zip bomb or a mistake.
pub const MAX_PARTS: usize = 16_384;

/// Any single part, decompressed. A 4 GB `content.xml` is not a document.
pub const MAX_PART_BYTES: u64 = 512 * 1024 * 1024;

/// Every part together, decompressed — what a zip bomb's amplification is measured against.
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// One entry's bytes, but never more than [`MAX_PART_BYTES`] whatever its header claimed, never
/// allocated from that claim, and an allocation failure is an error rather than an abort.
pub fn read_entry(entry: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let n = match entry.read(&mut chunk) {
            Ok(0) => return Ok(out),
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if out.len() as u64 + n as u64 > MAX_PART_BYTES {
            return Err(std::io::Error::other(format!(
                "decompresses to more than {MAX_PART_BYTES} bytes"
            )));
        }
        out.try_reserve(n)
            .map_err(|_| std::io::Error::other("not enough memory to decompress it"))?;
        out.extend_from_slice(&chunk[..n]);
    }
}
