// Copyright (c) 2026 The Project Symphonia Developers.
// SPDX-License-Identifier: MPL-2.0

//! ISO Common Encryption metadata. Payloads remain encrypted until the caller
//! explicitly decrypts them; the demuxer never substitutes a clear sample entry.

use crate::atoms::{Atom, AtomHeader, AtomIterator, AtomType, ReadAtom};
use symphonia_core::errors::{Result, decode_error, unsupported_error};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubsampleEncryption {
    pub clear_bytes: u32,
    pub encrypted_bytes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SampleEncryption {
    pub key_id: [u8; 16],
    pub iv: Vec<u8>,
    pub subsamples: Vec<SubsampleEncryption>,
}

#[derive(Clone, Debug)]
pub(crate) struct TrackEncryption {
    pub protected: bool,
    pub iv_size: u8,
    pub key_id: [u8; 16],
    pub constant_iv: Vec<u8>,
}

pub(crate) fn payload<R: ReadAtom>(
    it: &mut AtomIterator<R>,
    header: &AtomHeader,
) -> Result<Box<[u8]>> {
    let size = header.data_size().unwrap_or(u64::MAX);
    if size > 1024 * 1024 {
        return unsupported_error("isomp4: oversized encryption metadata");
    }
    Ok(it.read_boxed_slice_exact(size as usize)?)
}

pub(crate) struct ProtectionInfo(pub TrackEncryption);
impl Atom for ProtectionInfo {
    fn read<R: ReadAtom>(it: &mut AtomIterator<R>, _: &AtomHeader) -> crate::atoms::Result<Self> {
        let mut format = None;
        let mut scheme = None;
        let mut encryption = None;
        while let Some(header) = it.next_header()? {
            match header.atom_type() {
                AtomType::Other(kind) if &kind == b"frma" => {
                    let bytes = it.read_atom::<EncryptionData>()?.0;
                    if bytes.len() != 4 {
                        return crate::atoms::decode_error("isomp4: invalid frma");
                    }
                    format = Some(u32::from_be_bytes(bytes[..4].try_into().unwrap()));
                }
                AtomType::Other(kind) if &kind == b"schm" => {
                    let bytes = it.read_atom::<EncryptionData>()?.0;
                    if bytes.len() < 12 || bytes[0..3] != [0, 0, 0] || bytes[3] & !1 != 0 {
                        return crate::atoms::decode_error("isomp4: invalid schm header");
                    }
                    scheme = Some(u32::from_be_bytes(bytes[4..8].try_into().unwrap()));
                }
                AtomType::Other(kind) if &kind == b"schi" => {
                    encryption = it.read_atom::<SchemeInfo>()?.0;
                }
                _ => (),
            }
        }
        if !matches!(format, Some(v) if v == u32::from_be_bytes(*b"avc1") || v == u32::from_be_bytes(*b"mp4a"))
        {
            return crate::atoms::unsupported_error("isomp4: unsupported protected sample format");
        }
        if scheme != Some(u32::from_be_bytes(*b"cenc")) {
            return crate::atoms::unsupported_error("isomp4: unsupported encryption scheme");
        }
        Ok(encryption
            .map(ProtectionInfo)
            .ok_or(symphonia_core::errors::Error::DecodeError("isomp4: missing tenc"))?)
    }
}

struct SchemeInfo(Option<TrackEncryption>);
impl Atom for SchemeInfo {
    fn read<R: ReadAtom>(it: &mut AtomIterator<R>, _: &AtomHeader) -> crate::atoms::Result<Self> {
        let mut encryption = None;
        while let Some(header) = it.next_header()? {
            if header.atom_type() == AtomType::Other(*b"tenc") {
                if encryption.is_some() {
                    return crate::atoms::decode_error("isomp4: duplicate tenc");
                }
                encryption = Some(parse_tenc(&it.read_atom::<EncryptionData>()?.0)?);
            }
        }
        Ok(SchemeInfo(encryption))
    }
}

fn parse_tenc(data: &[u8]) -> Result<TrackEncryption> {
    if data.len() < 24 || data[0] > 1 || data[1..4] != [0, 0, 0] || data[4] != 0 || data[5] != 0 {
        return unsupported_error("isomp4: unsupported tenc version or encryption pattern");
    }
    if data[6] > 1 || !matches!(data[7], 0 | 8 | 16) {
        return decode_error("isomp4: invalid tenc defaults");
    }
    let protected = data[6] == 1;
    let iv_size = data[7];
    let key_id = data[8..24].try_into().unwrap();
    let constant_iv = if protected && iv_size == 0 {
        let size = *data
            .get(24)
            .ok_or(symphonia_core::errors::Error::DecodeError("isomp4: missing constant IV"))?
            as usize;
        if !matches!(size, 8 | 16) || data.len() != 25 + size {
            return decode_error("isomp4: invalid constant IV");
        }
        data[25..].to_vec()
    }
    else {
        if data.len() != 24 {
            return decode_error("isomp4: trailing tenc bytes");
        }
        Vec::new()
    };
    Ok(TrackEncryption { protected, iv_size, key_id, constant_iv })
}

pub(crate) fn parse_senc(
    data: &[u8],
    defaults: &TrackEncryption,
    expected: u32,
) -> Result<Vec<SampleEncryption>> {
    if data.len() < 8 || data[0] != 0 || data[1] != 0 || data[2] != 0 || data[3] & !2 != 0 {
        return unsupported_error("isomp4: unsupported senc header");
    }
    let count = u32::from_be_bytes(data[4..8].try_into().unwrap());
    if count != expected || count > 65536 {
        return decode_error("isomp4: senc sample count mismatch");
    }
    let mut position = 8usize;
    let mut entries = Vec::new();
    for _ in 0..count {
        let end = position + usize::from(defaults.iv_size);
        if end > data.len() {
            return decode_error("isomp4: truncated senc IV");
        }
        let iv = if defaults.iv_size == 0 {
            defaults.constant_iv.clone()
        }
        else {
            data[position..end].to_vec()
        };
        position = end;
        let mut subsamples = Vec::new();
        if data[3] & 2 != 0 {
            if position + 2 > data.len() {
                return decode_error("isomp4: missing senc subsample count");
            }
            let count =
                u16::from_be_bytes(data[position..position + 2].try_into().unwrap()) as usize;
            position += 2;
            if count > (data.len() - position) / 6 {
                return decode_error("isomp4: truncated senc subsamples");
            }
            for _ in 0..count {
                let clear_bytes =
                    u32::from(u16::from_be_bytes(data[position..position + 2].try_into().unwrap()));
                let encrypted_bytes =
                    u32::from_be_bytes(data[position + 2..position + 6].try_into().unwrap());
                position += 6;
                subsamples.push(SubsampleEncryption { clear_bytes, encrypted_bytes });
            }
        }
        entries.push(SampleEncryption { key_id: defaults.key_id, iv, subsamples });
    }
    if position != data.len() {
        return decode_error("isomp4: trailing senc bytes");
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preserves_iv_kid_and_subsample_boundaries() {
        let mut tenc = vec![0; 24];
        tenc[6] = 1;
        tenc[7] = 8;
        tenc[8..].fill(7);
        let defaults = parse_tenc(&tenc).unwrap();
        let mut senc = vec![0, 0, 0, 2, 0, 0, 0, 1];
        senc.extend_from_slice(&[9; 8]);
        senc.extend_from_slice(&[0, 2]);
        senc.extend_from_slice(&[0, 5, 0, 0, 0, 100, 0, 3, 0, 0, 0, 17]);
        let result = parse_senc(&senc, &defaults, 1).unwrap();
        assert_eq!(result[0].key_id, [7; 16]);
        assert_eq!(result[0].iv, [9; 8]);
        assert_eq!(
            result[0].subsamples,
            [
                SubsampleEncryption { clear_bytes: 5, encrypted_bytes: 100 },
                SubsampleEncryption { clear_bytes: 3, encrypted_bytes: 17 }
            ]
        );
        assert!(parse_senc(&senc, &defaults, 2).is_err());
        senc.pop();
        assert!(parse_senc(&senc, &defaults, 1).is_err());
    }
    #[test]
    fn validates_constant_iv_and_rejects_pattern_mode() {
        let mut tenc = vec![0; 24];
        tenc[6] = 1;
        tenc.extend_from_slice(&[8]);
        tenc.extend_from_slice(&[4; 8]);
        let defaults = parse_tenc(&tenc).unwrap();
        let samples = parse_senc(&[0, 0, 0, 0, 0, 0, 0, 1], &defaults, 1).unwrap();
        assert_eq!(samples[0].iv, [4; 8]);
        tenc[0] = 1;
        tenc[5] = 0x19;
        assert!(parse_tenc(&tenc).is_err());
        tenc[5] = 0;
        tenc.pop();
        assert!(parse_tenc(&tenc).is_err());
    }

    #[test]
    #[ignore = "requires the first four fragments of the public Shaka Sintel CENC AVC asset"]
    fn demuxes_clear_lead_and_protected_avc_without_modifying_payloads() {
        use symphonia_core::formats::{FormatOptions, FormatReader};
        use symphonia_core::io::{MediaSourceStream, ReadBytes};
        let path = std::env::var("SYMPHONIA_CENC_FIXTURE").unwrap();
        let bytes = std::fs::read(path).unwrap();
        let mut source =
            MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
        source.read_be_u32().unwrap();
        let mut reader = crate::IsoMp4Reader::try_new(source, FormatOptions::default())
            .unwrap_or_else(|e| {
                eprintln!("CENC_READER_ERROR {e:?}");
                panic!("CENC reader failed")
            });
        assert!(!reader.protection_headers().is_empty());
        let (mut clear, mut encrypted, mut sync) = (0, 0, 0);
        while let Some(packet) = reader.next_packet().unwrap_or_else(|e| {
            eprintln!("CENC_PACKET_ERROR {e:?} clear={clear} encrypted={encrypted}");
            panic!("CENC packet failed")
        }) {
            if let Some(info) = reader.packet_encryption() {
                assert_eq!(
                    info.key_id,
                    [
                        0x51, 0x74, 0x53, 0x86, 0x2d, 0x42, 0x56, 0xfd, 0x8b, 0xad, 0x4f, 0x58,
                        0x42, 0x20, 0x04, 0xd7
                    ]
                );
                assert_eq!(info.iv.len(), 8);
                assert_eq!(
                    info.subsamples
                        .iter()
                        .map(|s| u64::from(s.clear_bytes) + u64::from(s.encrypted_bytes))
                        .sum::<u64>(),
                    packet.data.len() as u64
                );
                encrypted += 1;
                sync += usize::from(reader.packet_is_sync());
            }
            else {
                clear += 1;
            }
        }
        eprintln!("CENC_COUNTS clear={clear} encrypted={encrypted} sync={sync}");
        assert_eq!((clear, encrypted, sync), (288, 96, 1));
    }
}

pub(crate) struct EncryptionData(pub Box<[u8]>);
impl Atom for EncryptionData {
    fn read<R: ReadAtom>(
        it: &mut AtomIterator<R>,
        header: &AtomHeader,
    ) -> crate::atoms::Result<Self> {
        Ok(Self(payload(it, header)?))
    }
}
