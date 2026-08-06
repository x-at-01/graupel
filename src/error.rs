use core::fmt;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Error {
    UnexpectedEnd,
    UnknownEncoding(u8),
    MalformedBlock,
    TooManyPoints(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnexpectedEnd => write!(f, "bit stream ended mid-value"),
            Error::UnknownEncoding(tag) => write!(f, "unknown encoding tag {tag}"),
            Error::MalformedBlock => write!(f, "bit stream describes an impossible value window"),
            Error::TooManyPoints(n) => {
                write!(f, "block holds {n} points, limit is {}", u32::MAX)
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

pub type Result<T> = core::result::Result<T, Error>;
