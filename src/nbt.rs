//! Just enough of the NBT format to read a world's name, mode and last play time from `level.dat`.

use std::io::Read;
use std::path::Path;

use anyhow::{Result, bail};

#[derive(Debug, Default)]
pub struct Level {
    pub name: Option<String>,
    /// Unix time in milliseconds.
    pub last_played: Option<u64>,
    pub game_type: Option<i32>,
}

pub fn read_level(path: &Path) -> Result<Level> {
    let raw = std::fs::read(path)?;
    let mut data = Vec::new();
    flate2::read::GzDecoder::new(&raw[..]).read_to_end(&mut data)?;
    let mut r = Reader { data: &data, pos: 0, level: Level::default(), depth: 0 };
    // Root: a named compound.
    if r.u8()? != 10 {
        bail!("level.dat does not start with a compound");
    }
    r.string()?;
    r.compound(&[])?;
    Ok(r.level)
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    level: Level,
    depth: u32,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        if self.pos + n > self.data.len() {
            bail!("level.dat ends early");
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into()?))
    }

    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into()?))
    }

    fn string(&mut self) -> Result<String> {
        let len = u16::from_be_bytes(self.take(2)?.try_into()?) as usize;
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }

    /// Reads a compound's entries; `path` names the compounds around it.
    fn compound(&mut self, path: &[&str]) -> Result<()> {
        loop {
            let tag = self.u8()?;
            if tag == 0 {
                return Ok(());
            }
            let name = self.string()?;
            let in_data = path == ["Data"];
            match (tag, name.as_str()) {
                (8, "LevelName") if in_data => self.level.name = Some(self.string()?),
                (4, "LastPlayed") if in_data => self.level.last_played = Some(self.i64()?.max(0) as u64),
                (3, "GameType") if in_data => self.level.game_type = Some(self.i32()?),
                (10, _) => {
                    let mut inner = path.to_vec();
                    inner.push(name.as_str());
                    self.nested(|r| r.compound(&inner))?;
                }
                _ => self.skip(tag)?,
            }
        }
    }

    fn nested(&mut self, f: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        self.depth += 1;
        if self.depth > 64 {
            bail!("level.dat nests too deep");
        }
        let r = f(self);
        self.depth -= 1;
        r
    }

    fn skip(&mut self, tag: u8) -> Result<()> {
        match tag {
            1 => drop(self.take(1)?),
            2 => drop(self.take(2)?),
            3 | 5 => drop(self.take(4)?),
            4 | 6 => drop(self.take(8)?),
            7 => {
                let n = self.i32()?.max(0) as usize;
                self.take(n)?;
            }
            8 => drop(self.string()?),
            9 => {
                let inner = self.u8()?;
                let n = self.i32()?.max(0);
                self.nested(|r| (0..n).try_for_each(|_| r.skip(inner)))?;
            }
            10 => self.nested(|r| r.compound(&["?"]))?,
            11 => {
                let n = self.i32()?.max(0) as usize;
                self.take(n * 4)?;
            }
            12 => {
                let n = self.i32()?.max(0) as usize;
                self.take(n * 8)?;
            }
            t => bail!("unknown NBT tag {t}"),
        }
        Ok(())
    }
}
