#[derive(Debug)]
pub(super) struct NbtCursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> NbtCursor<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    pub(super) fn skip(&mut self, len: usize) -> Result<(), ()> {
        if self.pos.saturating_add(len) > self.bytes.len() {
            return Err(());
        }
        self.pos += len;
        Ok(())
    }

    pub(super) fn read_u8(&mut self) -> Result<u8, ()> {
        if self.pos >= self.bytes.len() {
            return Err(());
        }
        let value = self.bytes[self.pos];
        self.pos += 1;
        Ok(value)
    }

    pub(super) fn read_u16(&mut self) -> Result<u16, ()> {
        let bytes = self.read_exact(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    pub(super) fn read_i32(&mut self) -> Result<i32, ()> {
        let bytes = self.read_exact(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(super) fn read_i64(&mut self) -> Result<i64, ()> {
        let bytes = self.read_exact(8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    pub(super) fn read_string(&mut self) -> Result<String, ()> {
        let len = self.read_u16()? as usize;
        let bytes = self.read_exact(len)?;
        Ok(String::from_utf8_lossy(bytes).to_string())
    }

    pub(super) fn read_exact(&mut self, len: usize) -> Result<&'a [u8], ()> {
        if self.pos.saturating_add(len) > self.bytes.len() {
            return Err(());
        }
        let start = self.pos;
        self.pos += len;
        Ok(&self.bytes[start..start + len])
    }
}
