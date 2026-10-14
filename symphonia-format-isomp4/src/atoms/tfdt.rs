// Copyright (c) 2026 The Project Symphonia Developers.
// SPDX-License-Identifier: MPL-2.0
use crate::atoms::{Atom, AtomHeader, AtomIterator, ReadAtom, Result, decode_error};
#[derive(Debug)]
pub struct TfdtAtom {
    pub base_media_decode_time: u64,
}
impl Atom for TfdtAtom {
    fn read<R: ReadAtom>(it: &mut AtomIterator<R>, _: &AtomHeader) -> Result<Self> {
        let (version, _) = it.read_extended_header()?;
        let base_media_decode_time = match version {
            0 => u64::from(it.read_u32()?),
            1 => it.read_u64()?,
            _ => return decode_error("isomp4 (tfdt): invalid version"),
        };
        Ok(Self {
            base_media_decode_time,
        })
    }
}
