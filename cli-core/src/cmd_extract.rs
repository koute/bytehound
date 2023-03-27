use crate::reader::parse_events;
use common::event::Event;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fs::File;
use std::path::PathBuf;

pub fn extract(input: PathBuf, output: PathBuf) -> Result<(), std::io::Error> {
    info!("Opening {:?}...", input);
    let fp = File::open(input)?;
    let (_, event_stream) = parse_events(fp)?;

    info!("Creating {:?} if it doesn't exist...", output);
    std::fs::create_dir_all(&output)?;

    let mut counter = HashMap::new();
    for event in event_stream {
        let event = match event {
            Ok(event) => event,
            Err(_) => break,
        };

        match event {
            Event::File { path, contents, .. } | Event::File64 { path, contents, .. } => {
                let mut relative_path = &*path;
                if relative_path.starts_with("/") {
                    relative_path = &relative_path[1..];
                }

                let mut target_path = output.join(relative_path);

                info!("Extracting {:?} into {:?}...", path, target_path);
                if let Some(parent) = target_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                match counter.entry(target_path.clone()) {
                    Entry::Vacant(bucket) => {
                        bucket.insert(0);
                    }
                    Entry::Occupied(mut bucket) => {
                        let parent = target_path.parent().unwrap();
                        let mut filename = target_path.file_name().unwrap().to_os_string();

                        if *bucket.get() == 0 {
                            let mut filename = filename.clone();
                            filename.push(".000");
                            std::fs::rename(&target_path, parent.join(filename))?;
                        }

                        filename.push(format!(".{:03}", bucket.get()));
                        target_path = parent.join(filename);

                        *bucket.get_mut() += 1;
                    }
                }

                std::fs::write(target_path, contents)?;
            }
            _ => {}
        }
    }

    Ok(())
}
