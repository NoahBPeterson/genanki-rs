use rusqlite::{Connection, Transaction, params};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;
use zip::{write::FileOptions, ZipWriter};

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use crate::apkg_schema::APKG_SCHEMA;
use crate::deck::Deck;
use crate::error::{database_error, json_error, zip_error};
use crate::Error;
use std::str::FromStr;
use crate::db_entries::{DeckDbEntry, ModelDbEntry};

/// Represents an entry in the 'config' table of an Anki collection.
#[derive(Debug, Clone)]
pub struct ConfigEntry {
    pub key: String,
    pub usn: i64,
    pub mtime_secs: i64,
    pub val: Vec<u8>,
}

/// Represents an entry in the 'deck_config' table of an Anki collection.
#[derive(Debug, Clone)]
pub struct DeckConfigEntry {
    pub id: i64,
    pub name: String,
    pub mtime_secs: i64,
    pub usn: i64,
    pub config_blob: Vec<u8>,
}

/// `Package` to pack `Deck`s and `media_files` and write them to a `.apkg` file
///
/// Example:
/// ```rust
/// use genanki_rs::{Package, Deck, Note, Model, Field, Template, ConfigEntry, DeckConfigEntry};
///
/// let model = Model::new(
///     1607392319,
///     "Simple Model",
///     vec![
///         Field::new("Question"),
///         Field::new("Answer"),
///         Field::new("MyMedia"),
///     ],
///     vec![Template::new("Card 1")
///         .qfmt("{{Question}}{{Question}}<br>{{MyMedia}}")
///         .afmt(r#"{{FrontSide}}<hr id="answer">{{Answer}}"#)],
/// );
///
/// let mut deck = Deck::new(1234, "Example Deck", "Example Deck with media");
/// deck.add_note(Note::new(model.clone(), vec!["What is the capital of France?", "Paris", "[sound:sound.mp3]"])?);
/// deck.add_note(Note::new(model.clone(), vec!["What is the capital of France?", "Paris", r#"<img src="image.jpg">"#])?);
///
/// let mut package = Package::new(vec![my_deck], vec!["sound.mp3", "images/image.jpg"])?;
/// package.write_to_file("output.apkg")?;
/// ```
pub struct Package {
    pub decks: Vec<Deck>,
    media_files: Vec<PathBuf>,
    configs: Vec<ConfigEntry>,
    deck_configs: Vec<DeckConfigEntry>,
}

impl Package {
    /// Create a new package with `decks` and `media_files`
    ///
    /// Returns `Err` if `media_files` are invalid
    pub fn new(decks: Vec<Deck>, media_files: Vec<&str>) -> Result<Self, Error> {
        let media_files = media_files
            .iter()
            .map(|&s| PathBuf::from_str(s))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { decks, media_files, configs: Vec::new(), deck_configs: Vec::new() })
    }

    /// Adds a configuration entry to the package.
    pub fn add_config_entry(&mut self, entry: ConfigEntry) {
        self.configs.push(entry);
    }

    /// Adds a deck configuration entry to the package.
    pub fn add_deck_config_entry(&mut self, entry: DeckConfigEntry) {
        self.deck_configs.push(entry);
    }

    /// Writes the package to any writer that implements Write and Seek
    pub fn write<W: Write + Seek>(&mut self, writer: W) -> Result<(), Error> {
        self.write_maybe_timestamp(writer, None)
    }

    /// Writes the package to any writer that implements Write and Seek using a timestamp
    pub fn write_timestamp<W: Write + Seek>(
        &mut self,
        writer: W,
        timestamp: f64,
    ) -> Result<(), Error> {
        self.write_maybe_timestamp(writer, Some(timestamp))
    }

    /// Writes the package to a file
    ///
    /// Returns `Err` if the `file` cannot be created
    pub fn write_to_file(&mut self, file: &str) -> Result<(), Error> {
        let file = File::create(file)?;
        self.write_maybe_timestamp(file, None)
    }

    /// Writes the package to a file using a timestamp
    ///
    /// Returns `Err` if the `file` cannot be created
    pub fn write_to_file_timestamp(&mut self, file: &str, timestamp: f64) -> Result<(), Error> {
        let file = File::create(file)?;
        self.write_maybe_timestamp(file, Some(timestamp))
    }

    fn write_maybe_timestamp<W: Write + Seek>(
        &mut self,
        writer: W,
        timestamp_opt: Option<f64>,
    ) -> Result<(), Error> {
        let db_file = NamedTempFile::new()?.into_temp_path();
        let mut conn = Connection::open(&db_file).map_err(database_error)?;
        let transaction = conn.transaction().map_err(database_error)?;

        let timestamp_sec = timestamp_opt
            .unwrap_or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64());

        self.write_schema_and_col_table(&transaction, timestamp_sec)?;
        self.write_deck_content_data(&transaction, timestamp_sec)?;

        transaction.commit().map_err(database_error)?;
        conn.close().map_err(|(_, e)| database_error(e)).expect("Should always close");

        let mut outzip = ZipWriter::new(writer);
        outzip
            .start_file("collection.anki2", FileOptions::default())
            .map_err(zip_error)?;
        outzip.write_all(&read_file_bytes(db_file)?)?;

        let media_file_idx_to_path = self
            .media_files
            .iter()
            .enumerate()
            .collect::<HashMap<usize, &PathBuf>>();
        let media_map = media_file_idx_to_path
            .clone()
            .into_iter()
            .map(|(id, path)| {
                (
                    id.to_string(),
                    path.file_name()
                        .expect("Should always have a filename")
                        .to_str()
                        .expect("should always have string"),
                )
            })
            .collect::<HashMap<String, &str>>();
        let media_json = serde_json::to_string(&media_map).map_err(json_error)?;
        outzip
            .start_file("media", FileOptions::default())
            .map_err(zip_error)?;
        outzip.write_all(media_json.as_bytes())?;

        for (idx, &path) in &media_file_idx_to_path {
            outzip
                .start_file(idx.to_string(), FileOptions::default())
                .map_err(zip_error)?;
            outzip.write_all(&read_file_bytes(path)?)?;
        }
        outzip.finish().map_err(zip_error)?;
        Ok(())
    }

    fn write_schema_and_col_table(&self, transaction: &Transaction, timestamp_sec: f64) -> Result<(), Error> {
        transaction.execute_batch(APKG_SCHEMA).map_err(database_error)?;

        // Write config table entries
        for config_entry in &self.configs {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO config (key, usn, mtime_secs, val) VALUES (?, ?, ?, ?)",
                    params![
                        config_entry.key,
                        config_entry.usn,
                        config_entry.mtime_secs,
                        config_entry.val
                    ],
                )
                .map_err(database_error)?;
        }

        // Write deck_config table entries
        for deck_config_entry in &self.deck_configs {
            transaction
                .execute(
                    "INSERT OR REPLACE INTO deck_config (id, name, mtime_secs, usn, config) VALUES (?, ?, ?, ?, ?)",
                    params![
                        deck_config_entry.id,
                        deck_config_entry.name,
                        deck_config_entry.mtime_secs,
                        deck_config_entry.usn,
                        deck_config_entry.config_blob
                    ],
                )
                .map_err(database_error)?;
        }

        let crt_val = timestamp_sec as i64;
        let mod_val = (timestamp_sec * 1000.0) as i64;
        let scm_val = mod_val;

        let mut models_map_for_col: HashMap<String, ModelDbEntry> = HashMap::new();
        for deck_item in &self.decks {
            for note in deck_item.notes() { 
                let mut model_clone = note.model().clone(); 
                let model_id_str = model_clone.id.to_string();
                if !models_map_for_col.contains_key(&model_id_str) {
                    models_map_for_col.insert(model_id_str, model_clone.to_model_db_entry(timestamp_sec, deck_item.id)?);
                }
            }
        }
        let models_json_str = serde_json::to_string(&models_map_for_col).map_err(json_error)?;

        let mut decks_map_for_col: HashMap<String, DeckDbEntry> = HashMap::new();
        for deck_item in &self.decks {
            decks_map_for_col.insert(deck_item.id.to_string(), deck_item.to_deck_db_entry());
        }
        
        if !decks_map_for_col.contains_key("1") {
            let default_deck = Deck::new(1, "Default", "");
            decks_map_for_col.insert("1".to_string(), default_deck.to_deck_db_entry());
        }
        let decks_json_str = serde_json::to_string(&decks_map_for_col).map_err(json_error)?;
        
        let default_conf_json = "{\"activeDecks\": [1], \"addToCur\": true, \"collapseTime\": 1200, \"curDeck\": 1, \"curModel\": \"1607392319\", \"dueCounts\": true, \"estTimes\": true, \"newBury\": true, \"newSpread\": 0, \"nextPos\": 1, \"sortBackwards\": false, \"sortType\": \"noteFld\", \"timeLim\": 0}";
        let default_dconf_json = "{\"1\": {\"autoplay\": true, \"id\": 1, \"lapse\": {\"delays\": [10], \"leechAction\": 0, \"leechFails\": 8, \"minInt\": 1, \"mult\": 0}, \"maxTaken\": 60, \"mod\": 0, \"name\": \"Default\", \"new\": {\"bury\": true, \"delays\": [1, 10], \"initialFactor\": 2500, \"ints\": [1, 4, 7], \"order\": 1, \"perDay\": 20, \"separate\": true}, \"replayq\": true, \"rev\": {\"bury\": true, \"ease4\": 1.3, \"fuzz\": 0.05, \"ivlFct\": 1, \"maxIvl\": 36500, \"minSpace\": 1, \"perDay\": 100}, \"timer\": 0, \"usn\": 0}}";

        transaction.execute(
            "INSERT INTO col (id, crt, mod, scm, ver, dty, usn, ls, conf, models, decks, dconf, tags) VALUES (NULL, ?, ?, ?, 11, 0, -1, 0, ?, ?, ?, ?, ?)",
            params![
                crt_val,
                mod_val,
                scm_val,
                default_conf_json,
                models_json_str,
                decks_json_str,
                default_dconf_json,
                "{}"
            ],
        ).map_err(database_error)?;
        Ok(())
    }

    fn write_deck_content_data(&mut self, transaction: &Transaction, timestamp_sec: f64) -> Result<(), Error> {
        let mut id_gen = ((timestamp_sec * 1000.0) as usize)..;
        for deck in &mut self.decks {
            deck.write_notes_and_cards_to_db(&transaction, timestamp_sec, &mut id_gen)?;
        }
        Ok(())
    }
}

fn read_file_bytes<P: AsRef<Path>>(path: P) -> Result<Vec<u8>, Error> {
    let mut handle = File::open(path)?;
    let mut data = Vec::new();
    handle.read_to_end(&mut data)?;
    Ok(data)
}
