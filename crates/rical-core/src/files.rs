use std::{fs::read_dir, path::PathBuf};

use log::warn;

pub fn paths_to_files(paths: &[PathBuf], recursive: bool) -> Vec<PathBuf> {
    paths
        .iter()
        .flat_map(|path| {
            if path.is_file() {
                vec![path.clone()]
            } else if path.is_dir() {
                read_dir(path)
                    .map(|rd| {
                        rd.into_iter()
                            .filter_map(|e| e.ok())
                            .flat_map(|e| {
                                e.metadata()
                                    .map(|m| {
                                        if m.is_file() {
                                            vec![e.path()]
                                        } else if m.is_dir() && recursive {
                                            paths_to_files(&[e.path()], recursive)
                                        } else {
                                            Vec::new()
                                        }
                                    })
                                    .unwrap_or_default()
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else {
                warn!("Path was neither file or directory: {:?}", path.metadata());
                Vec::new()
            }
        })
        .collect()
}
