// Symphonia
// Copyright (c) 2019-2026 The Project Symphonia Developers.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use symphonia_core::errors::{
    Error, Result, SeekErrorKind, decode_error, seek_error, unsupported_error,
};
use symphonia_core::packet::Packet;
use symphonia_core::units::{Duration, Timestamp};

use crate::atoms::ElstAtom;

/// A contiguous, unit-rate media edit, optionally preceded by an empty edit.
/// Encoded preroll is retained; packet trim metadata describes the valid window.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EditTimeline {
    offset: i64,
    start: i64,
    end: Option<i64>,
    edited: bool,
}

impl EditTimeline {
    pub(crate) fn new(
        edits: Option<&ElstAtom>,
        movie_scale: u32,
        media_scale: u32,
    ) -> Result<Self> {
        let Some(edits) = edits
        else {
            return Ok(Self::default());
        };
        Self::from_entries(
            edits.entries.iter().map(|entry| {
                (
                    entry.segment_duration,
                    entry.media_time,
                    entry.media_rate_int,
                    entry.media_rate_frac,
                )
            }),
            movie_scale,
            media_scale,
        )
    }

    fn from_entries(
        mut entries: impl Iterator<Item = (u64, i64, i16, i16)>,
        movie_scale: u32,
        media_scale: u32,
    ) -> Result<Self> {
        let Some(first) = entries.next()
        else {
            return Ok(Self::default());
        };
        let (empty, media) = if first.1 == -1 {
            if first.2 != 1 || first.3 != 0 {
                return unsupported_error("isomp4: non-unit edit rate");
            }
            (first.0, entries.next().ok_or(Error::Unsupported("isomp4: empty-only edit list"))?)
        }
        else {
            (0, first)
        };
        if entries.next().is_some() {
            return unsupported_error("isomp4: multiple media edits");
        }
        if media.1 < 0 {
            return decode_error("isomp4: invalid edit media time");
        }
        if media.2 != 1 || media.3 != 0 {
            return unsupported_error("isomp4: non-unit edit rate");
        }
        let start = i64::try_from(rescale(empty, movie_scale, media_scale)?)
            .map_err(|_| Error::DecodeError("isomp4: edit start overflow"))?;
        let offset =
            start.checked_sub(media.1).ok_or(Error::DecodeError("isomp4: edit offset overflow"))?;
        // Zero duration is used by fragmented initialization segments whose
        // final duration is not known yet. Keep the existing duration fallback.
        let end = if media.0 == 0 {
            None
        }
        else {
            let duration = empty
                .checked_add(media.0)
                .ok_or(Error::DecodeError("isomp4: edit duration overflow"))?;
            Some(
                i64::try_from(rescale(duration, movie_scale, media_scale)?)
                    .map_err(|_| Error::DecodeError("isomp4: edit duration overflow"))?,
            )
        };
        Ok(Self { offset, start, end, edited: true })
    }

    pub(crate) fn has_edits(self) -> bool {
        self.edited
    }

    pub(crate) fn duration(self) -> Option<Duration> {
        self.end.map(|end| Duration::new(end as u64))
    }

    pub(crate) fn media_duration(self) -> Option<Duration> {
        self.end.map(|end| Duration::new((end - self.start) as u64))
    }

    pub(crate) fn map(self, ts: Timestamp) -> Result<Timestamp> {
        ts.get()
            .checked_add(self.offset)
            .map(Timestamp::new)
            .ok_or(Error::DecodeError("isomp4: edited timestamp overflow"))
    }

    pub(crate) fn media_seek_ts(self, ts: Timestamp) -> Result<Timestamp> {
        if ts.is_negative() || self.end.is_some_and(|end| ts.get() >= end) {
            return seek_error(SeekErrorKind::OutOfRange);
        }
        // Seeking within an empty edit selects the first media edit. No
        // synthetic samples or timestamps are generated for the empty span.
        ts.get()
            .max(self.start)
            .checked_sub(self.offset)
            .filter(|ts| *ts >= 0)
            .map(Timestamp::new)
            .ok_or(Error::SeekError(SeekErrorKind::OutOfRange))
    }

    pub(crate) fn trim_packet(self, packet: &mut Packet) {
        if !self.edited {
            return;
        }
        let pts = i128::from(packet.pts.get());
        let block = i128::from(packet.dur.get());
        let trim_start = (i128::from(self.start) - pts).clamp(0, block);
        let trim_end =
            self.end.map_or(0, |end| (pts + block - i128::from(end)).clamp(0, block - trim_start));
        packet.trim_start = Duration::new(trim_start as u64);
        packet.trim_end = Duration::new(trim_end as u64);
        packet.dur = Duration::new((block - trim_start - trim_end) as u64);
    }
}

/// Round to the nearest media tick using bounded integer arithmetic. An edit
/// in movie units need not be an integral number of media ticks.
fn rescale(value: u64, from: u32, to: u32) -> Result<u64> {
    if from == 0 || to == 0 {
        return decode_error("isomp4: zero edit timescale");
    }
    let value = (u128::from(value) * u128::from(to) + u128::from(from / 2)) / u128::from(from);
    value.try_into().map_err(|_| Error::DecodeError("isomp4: edit timescale overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(entries: &[(u64, i64, i16, i16)]) -> Result<EditTimeline> {
        EditTimeline::from_entries(entries.iter().copied(), 1000, 16000)
    }

    #[test]
    fn composition_delay_maps_pts_and_signed_dts_to_movie_time() {
        let timeline = edit(&[(3000, 4000, 1, 0)]).unwrap();
        assert_eq!(timeline.map(Timestamp::new(4000)).unwrap(), Timestamp::ZERO);
        assert_eq!(timeline.map(Timestamp::ZERO).unwrap(), Timestamp::new(-4000));
        assert_eq!(timeline.duration(), Some(Duration::new(48000)));
        assert_eq!(timeline.media_duration(), Some(Duration::new(48000)));
        for movie in [0, 1, 8000, 16000, 40000] {
            let media = timeline.media_seek_ts(Timestamp::new(movie)).unwrap();
            assert_eq!(timeline.map(media).unwrap(), Timestamp::new(movie));
        }
        assert!(timeline.media_seek_ts(Timestamp::new(48000)).is_err());
    }

    #[test]
    fn leading_empty_edit_is_not_encoded_media() {
        let timeline = edit(&[(500, -1, 1, 0), (3000, 4000, 1, 0)]).unwrap();
        assert_eq!(timeline.map(Timestamp::new(4000)).unwrap(), Timestamp::new(8000));
        assert_eq!(timeline.duration(), Some(Duration::new(56000)));
        assert_eq!(timeline.media_duration(), Some(Duration::new(48000)));
        assert_eq!(timeline.media_seek_ts(Timestamp::ZERO).unwrap(), Timestamp::new(4000));
        assert_eq!(timeline.media_seek_ts(Timestamp::new(12000)).unwrap(), Timestamp::new(8000));
    }

    #[test]
    fn retained_preroll_and_padding_have_bounded_trim_metadata() {
        let timeline = edit(&[(1000, 4000, 1, 0)]).unwrap();
        for (pts, start, end, valid) in [
            (-1000, 1000, 0, 0),
            (-500, 500, 0, 500),
            (0, 0, 0, 1000),
            (15500, 0, 500, 500),
            (17000, 0, 1000, 0),
        ] {
            let mut packet = Packet::new(1, Timestamp::new(pts), Duration::new(1000), vec![1, 2]);
            packet.dts = Timestamp::new(pts - 1000);
            timeline.trim_packet(&mut packet);
            assert_eq!(
                (packet.trim_start.get(), packet.trim_end.get(), packet.dur.get()),
                (start, end, valid)
            );
            assert_eq!(packet.block_dur(), Duration::new(1000));
            assert_eq!(packet.pts, Timestamp::new(pts));
            assert_eq!(packet.dts, Timestamp::new(pts - 1000));
            assert_eq!(&*packet.data, &[1, 2]);
        }
    }

    #[test]
    fn unknown_fragment_duration_and_absent_edits_preserve_duration_fallback() {
        let timeline = edit(&[(0, 4000, 1, 0)]).unwrap();
        assert_eq!(timeline.duration(), None);
        assert_eq!(timeline.map(Timestamp::new(4000)).unwrap(), Timestamp::ZERO);
        let identity = EditTimeline::new(None, 1000, 16000).unwrap();
        assert_eq!(identity.map(Timestamp::new(-4)).unwrap(), Timestamp::new(-4));
        assert_eq!(identity.duration(), None);
    }

    #[test]
    fn unsupported_edit_shapes_and_rates_are_rejected() {
        assert!(edit(&[(1000, -1, 1, 0)]).is_err());
        assert!(edit(&[(1000, -2, 1, 0)]).is_err());
        assert!(edit(&[(1000, 0, 0, 0)]).is_err());
        assert!(edit(&[(1000, 0, 1, 1)]).is_err());
        assert!(edit(&[(1000, 0, 1, 0), (1000, 16000, 1, 0)]).is_err());
        assert!(edit(&[(100, -1, 1, 0), (1000, 0, 1, 0), (1000, 16000, 1, 0)]).is_err());
    }

    #[test]
    fn rescale_and_timestamp_overflow_are_checked() {
        assert_eq!(rescale(1, 1000, 44100).unwrap(), 44);
        assert_eq!(rescale(5, 1000, 44100).unwrap(), 221);
        assert!(rescale(1, 0, 48000).is_err());
        assert!(rescale(u64::MAX, 1, u32::MAX).is_err());
        assert!(edit(&[(u64::MAX, 0, 1, 0)]).is_err());
        let timeline = edit(&[(1000, 4000, 1, 0)]).unwrap();
        assert!(timeline.map(Timestamp::MIN).is_err());
    }
}
