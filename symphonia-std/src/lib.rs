#![forbid(unsafe_code)]
#![no_std]

extern crate alloc;

pub use core::{
    cmp, convert, default, error, f32, f64, fmt, hash, i8, i16, i32, i64, marker, mem, num, ops,
    result, str, u8, u16, u32, u64,
};

pub mod borrow {
    pub use alloc::borrow::{Cow, ToOwned};
}

pub mod collections {
    pub use alloc::collections::{BTreeMap, BTreeMap as HashMap, VecDeque};
}

pub mod fs {
    use super::io;

    pub struct File;

    pub struct Metadata {
        len: u64,
        is_file: bool,
    }

    impl Metadata {
        pub fn len(&self) -> u64 {
            self.len
        }

        pub fn is_file(&self) -> bool {
            self.is_file
        }
    }

    impl File {
        pub fn metadata(&self) -> io::Result<Metadata> {
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    impl io::Read for File {
        fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
            Err(io::ErrorKind::Unsupported.into())
        }
    }

    impl io::Seek for File {
        fn seek(&mut self, _pos: io::SeekFrom) -> io::Result<u64> {
            Err(io::ErrorKind::Unsupported.into())
        }
    }
}

pub mod io {
    use core::fmt;
    use core::ops::{Deref, DerefMut};

    pub type Result<T> = core::result::Result<T, Error>;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ErrorKind {
        UnexpectedEof,
        Interrupted,
        InvalidData,
        Unsupported,
        Other,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct Error {
        kind: ErrorKind,
    }

    impl Error {
        pub fn new<T>(kind: ErrorKind, _error: T) -> Self {
            Self { kind }
        }

        pub fn other<T>(_error: T) -> Self {
            Self {
                kind: ErrorKind::Other,
            }
        }

        pub fn kind(&self) -> ErrorKind {
            self.kind
        }
    }

    impl From<ErrorKind> for Error {
        fn from(kind: ErrorKind) -> Self {
            Self { kind }
        }
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("io error")
        }
    }

    impl core::error::Error for Error {}

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum SeekFrom {
        Start(u64),
        End(i64),
        Current(i64),
    }

    pub trait Read {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize>;

        fn read_exact(&mut self, mut buf: &mut [u8]) -> Result<()> {
            while !buf.is_empty() {
                match self.read(buf) {
                    Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
                    Ok(n) => {
                        let tmp = buf;
                        buf = &mut tmp[n..];
                    }
                    Err(err) => return Err(err),
                }
            }
            Ok(())
        }

        fn read_vectored(&mut self, bufs: &mut [IoSliceMut<'_>]) -> Result<usize> {
            for buf in bufs {
                if !buf.is_empty() {
                    return self.read(buf);
                }
            }
            Ok(0)
        }
    }

    pub trait Seek {
        fn seek(&mut self, pos: SeekFrom) -> Result<u64>;

        fn stream_position(&mut self) -> Result<u64> {
            self.seek(SeekFrom::Current(0))
        }
    }

    pub struct IoSliceMut<'a> {
        inner: &'a mut [u8],
    }

    impl<'a> IoSliceMut<'a> {
        pub fn new(buf: &'a mut [u8]) -> Self {
            Self { inner: buf }
        }
    }

    impl Deref for IoSliceMut<'_> {
        type Target = [u8];

        fn deref(&self) -> &Self::Target {
            self.inner
        }
    }

    impl DerefMut for IoSliceMut<'_> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            self.inner
        }
    }

    pub struct Cursor<T> {
        inner: T,
        pos: u64,
    }

    impl<T> Cursor<T> {
        pub fn new(inner: T) -> Self {
            Self { inner, pos: 0 }
        }

        pub fn get_ref(&self) -> &T {
            &self.inner
        }

        pub fn position(&self) -> u64 {
            self.pos
        }
    }

    impl<T: AsRef<[u8]>> Read for Cursor<T> {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            let data = self.inner.as_ref();
            let start = core::cmp::min(self.pos as usize, data.len());
            let len = core::cmp::min(buf.len(), data.len() - start);
            buf[..len].copy_from_slice(&data[start..start + len]);
            self.pos += len as u64;
            Ok(len)
        }
    }

    impl<T: AsRef<[u8]>> Seek for Cursor<T> {
        fn seek(&mut self, pos: SeekFrom) -> Result<u64> {
            let len = self.inner.as_ref().len() as i128;
            let next = match pos {
                SeekFrom::Start(pos) => pos as i128,
                SeekFrom::End(pos) => len + pos as i128,
                SeekFrom::Current(pos) => self.pos as i128 + pos as i128,
            };
            if next < 0 {
                return Err(ErrorKind::InvalidData.into());
            }
            self.pos = next as u64;
            Ok(self.pos)
        }
    }

    impl Read for &[u8] {
        fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
            let len = core::cmp::min(buf.len(), self.len());
            buf[..len].copy_from_slice(&self[..len]);
            *self = &self[len..];
            Ok(len)
        }
    }
}

pub mod prelude {
    pub mod v1 {
        pub use alloc::boxed::Box;
        pub use alloc::borrow::ToOwned;
        pub use alloc::string::{String, ToString};
        pub use alloc::vec;
        pub use alloc::vec::Vec;

        pub trait FloatExt {
            fn sin(self) -> Self;
            fn cos(self) -> Self;
            fn sqrt(self) -> Self;
            fn floor(self) -> Self;
            fn round(self) -> Self;
            fn powf(self, n: Self) -> Self;
        }

        impl FloatExt for f32 {
            fn sin(self) -> Self {
                libm::sinf(self)
            }

            fn cos(self) -> Self {
                libm::cosf(self)
            }

            fn sqrt(self) -> Self {
                libm::sqrtf(self)
            }

            fn floor(self) -> Self {
                libm::floorf(self)
            }

            fn round(self) -> Self {
                libm::roundf(self)
            }

            fn powf(self, n: Self) -> Self {
                libm::powf(self, n)
            }
        }

        impl FloatExt for f64 {
            fn sin(self) -> Self {
                libm::sin(self)
            }

            fn cos(self) -> Self {
                libm::cos(self)
            }

            fn sqrt(self) -> Self {
                libm::sqrt(self)
            }

            fn floor(self) -> Self {
                libm::floor(self)
            }

            fn round(self) -> Self {
                libm::round(self)
            }

            fn powf(self, n: Self) -> Self {
                libm::pow(self, n)
            }
        }
    }
}

pub mod sync {
    pub use alloc::sync::Arc;
}
