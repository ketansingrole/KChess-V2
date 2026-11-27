//! Sorted runs on disk, for an import that gathers more than it holds in memory: each time memory
//! is full it writes what it holds out sorted as a run, and at its end reads every run back merged
//! into one sequence in order.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Seek as _};
use std::path::Path;

use kchess_contracts::error::{CoreError, Result};

pub struct Runs {
    files: Vec<File>,
    temp: tempfile::TempDir,
    io: fn(std::io::Error) -> CoreError,
}

impl Runs {
    /// Run files go in a private directory inside `parent` (the profile's databases folder, on the
    /// same disk as the database) named from `prefix`, removed with the runs; `io` says what
    /// failed when the disk does.
    pub fn new(parent: &Path, prefix: &str, io: fn(std::io::Error) -> CoreError) -> Result<Runs> {
        Ok(Runs {
            files: Vec::new(),
            temp: tempfile::Builder::new()
                .prefix(prefix)
                .tempdir_in(parent)
                .map_err(io)?,
            io,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// One run, which `write` writes in order.
    pub fn spill(
        &mut self,
        write: impl FnOnce(&mut BufWriter<File>) -> std::io::Result<()>,
    ) -> Result<()> {
        let io = self.io;
        let path = self.temp.path().join(format!("run-{}", self.files.len()));
        let file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(io)?;
        let mut out = BufWriter::with_capacity(1 << 20, file);
        write(&mut out).map_err(io)?;
        let mut file = out.into_inner().map_err(|e| io(e.into_error()))?;
        file.rewind().map_err(io)?;
        self.files.push(file);
        Ok(())
    }

    /// Every run's records in order, to `each`; `read` reads a run's next record, none at its end.
    pub fn merge<T: Ord>(
        self,
        read: impl Fn(&mut BufReader<File>) -> std::io::Result<Option<T>>,
        mut each: impl FnMut(T) -> Result<()>,
    ) -> Result<()> {
        let io = self.io;
        let mut readers: Vec<_> = self
            .files
            .into_iter()
            .map(|file| BufReader::with_capacity(1 << 18, file))
            .collect();
        let mut heap = BinaryHeap::new();
        for (index, reader) in readers.iter_mut().enumerate() {
            if let Some(record) = read(reader).map_err(io)? {
                heap.push(Reverse((record, index)));
            }
        }
        while let Some(Reverse((record, index))) = heap.pop() {
            if let Some(reader) = readers.get_mut(index)
                && let Some(following) = read(reader).map_err(io)?
            {
                heap.push(Reverse((following, index)));
            }
            each(record)?;
        }
        Ok(())
    }
}
