// SPDX-FileCopyrightText: 2026 Florian Wilhelm <fwilhelm.wgt+github@gmail.com>
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! XML in both directions: the tolerant walker every reader uses ([`read`]) and the one writer
//! every writer uses ([`write`](mod@write)), which is where well-formedness is guaranteed once.

pub mod read;
pub mod write;
