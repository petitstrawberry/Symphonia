// Copyright (c) 2026 The Project Symphonia Developers.
// SPDX-License-Identifier: MPL-2.0
use crate::atoms::{Atom, AtomHeader, AtomIterator, ReadAtom, Result, decode_error};
#[derive(Debug)]
pub struct CttsAtom {
    entries: Vec<(u32, i64)>,
}
impl CttsAtom {
    pub fn offset(&self, mut sample: u32) -> Option<i64> {
        for &(count, offset) in &self.entries {
            if sample < count {
                return Some(offset);
            }
            sample -= count;
        }
        None
    }
}
impl Atom for CttsAtom {
    fn read<R: ReadAtom>(it: &mut AtomIterator<R>, _: &AtomHeader) -> Result<Self> {
        let (version, _) = it.read_extended_header()?;
        if version > 1 {
            return decode_error("isomp4 (ctts): invalid version");
        }
        let count = it.read_u32()?;
        let mut entries = Vec::new();
        for _ in 0..count {
            let samples = it.read_u32()?;
            let raw = it.read_u32()?;
            if samples == 0 {
                return decode_error("isomp4 (ctts): empty entry");
            }
            entries.push((
                samples,
                if version == 1 {
                    i64::from(raw as i32)
                } else {
                    i64::from(raw)
                },
            ));
        }
        Ok(Self { entries })
    }
}
