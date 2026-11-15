//! Received match data and HTTP submissions, independent of terminal diagnostics.
use anyhow::{Context, Result};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct MatchRecord {
    pub directory: PathBuf,
    http: File,
    observations: File,
}

impl MatchRecord {
    pub fn create(
        root: &Path,
        problem: &Value,
        metadata: &Value,
        timestamp_ms: u64,
    ) -> Result<Self> {
        fs::create_dir_all(root).context("Failed to create the match record root")?;
        let mut sequence = 0u64;
        let directory = loop {
            let path = root.join(format!("{timestamp_ms}-{}-{sequence}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => break path,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => sequence += 1,
                Err(error) => {
                    return Err(error).context("Failed to create a unique match directory");
                }
            }
        };
        // Each match owns a new directory, including when the same map is played again.
        write_json(&directory.join("metadata.json"), metadata)?;
        write_json(&directory.join("problem.json"), problem)?;
        write_json(&directory.join("map.json"), &problem["map"])?;
        let http = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("http.jsonl"))?;
        let observations = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("observations.jsonl"))?;
        Ok(Self {
            directory,
            http,
            observations,
        })
    }

    pub fn append(&mut self, event: &Value) -> Result<()> {
        let target = if event["event"] == "observation" {
            &mut self.observations
        } else {
            &mut self.http
        };
        let mut bytes = serde_json::to_vec(event)?;
        bytes.push(b'\n');
        // No buffered writer: previously completed records survive process termination.
        target
            .write_all(&bytes)
            .context("Failed to append match data")
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    fs::write(path, bytes).with_context(|| format!("Failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_matches_with_the_same_timestamp_never_overwrite() {
        let root = std::env::temp_dir().join(format!("bridge-record-test-{}", std::process::id()));
        let problem = serde_json::json!({"map": {"cells": [[0, 1]]}, "extra": [1, 2]});
        let mut first = MatchRecord::create(&root, &problem, &serde_json::json!({}), 123).unwrap();
        first.append(&serde_json::json!({"event":"observation", "state":{"traffics":[{"pos":1,"status":2}]}})).unwrap();
        let second = MatchRecord::create(&root, &problem, &serde_json::json!({}), 123).unwrap();
        assert_ne!(first.directory, second.directory);
        let saved: Value =
            serde_json::from_slice(&fs::read(first.directory.join("problem.json")).unwrap())
                .unwrap();
        assert_eq!(saved, problem);
        assert!(
            fs::read_to_string(first.directory.join("observations.jsonl"))
                .unwrap()
                .contains("traffics")
        );
        drop((first, second));
        fs::remove_dir_all(root).unwrap();
    }
}
