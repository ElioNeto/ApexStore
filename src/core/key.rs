#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct KeySlice<'a>(&'a [u8]);

impl<'a> KeySlice<'a> {
    pub fn new(slice: &'a [u8]) -> Self {
        Self(slice)
    }

    pub fn as_slice(&self) -> &'a [u8] {
        self.0
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl<'a> std::ops::Deref for KeySlice<'a> {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'a> From<&'a [u8]> for KeySlice<'a> {
    fn from(slice: &'a [u8]) -> Self {
        Self(slice)
    }
}

impl<'a> From<&'a Vec<u8>> for KeySlice<'a> {
    fn from(vec: &'a Vec<u8>) -> Self {
        Self(vec.as_slice())
    }
}
