use rusqlite::{Connection, Transaction, params};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;
use zip::{write::FileOptions, ZipWriter};

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use log::info;

use crate::apkg_schema::{APKG_SCHEMA, APKG_SCHEMA_V11, APKG_SCHEMA_FIELDS};
use crate::apkg_col::APKG_COL;
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

/// Added DeckInfoEntry struct
#[derive(Debug, Clone)]
pub struct DeckInfoEntry {
    pub id: i64,
    pub name: String,
    pub mtime_secs: i64,
    pub usn: i64,
    pub common: Vec<u8>, // Expects JSON blob
    pub kind: Vec<u8>,   // Expects JSON blob
}

/// Added NotetypeEntry struct
#[derive(Debug, Clone)]
pub struct NotetypeEntry {
    pub id: i64, // Note type ID (Primary Key). Timestamp in ms.
    pub name: String, // Name of the note type.
    pub mtime_secs: i64, // Modification timestamp (seconds).
    pub usn: i64, // Update Sequence Number.
    pub config: Vec<u8>, // JSON blob containing sort field, CSS, etc.
}

/// the location of the media files, either as a path on the filesystem or as bytes from memory
pub enum MediaFile {
    /// a path on the filesystem
    Path(PathBuf),
    /// bytes of the file and a filename
    Bytes(Vec<u8>, String),
}
impl MediaFile {
    /// Create a new `MediaFile` from a path on the filesystem
    pub fn new_from_file<P: AsRef<Path>>(path: P) -> Self {
        Self::Path(path.as_ref().to_path_buf())
    }

    /// Create a new `MediaFile` from a path on the filesystem using a `&str`
    pub fn new_from_file_path(path: &str) -> Result<Self, Error> {
        Ok(Self::Path(PathBuf::from_str(path)?))
    }

    /// Create a new `MediaFile` from bytes from memory and a filename
    pub fn new_from_bytes(bytes: &[u8], name: &str) -> Self {
        Self::Bytes(bytes.to_vec(), name.to_owned())
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldEntry {
    pub ntid: i64,
    pub ord: i64,
    pub name: String,
    pub config: Vec<u8>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TemplateEntry {
    pub ntid: i64,
    pub ord: i64,
    pub name: String,
    pub mtime_secs: i64,
    pub usn: i64,
    pub config: Vec<u8>, // JSON blob
}

/// `Package` to pack `Deck`s and `media_files` and write them to a `.apkg` file
///
/// # Example (media files on the filesystem):
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

/// Graves table entry (deleted items tombstone)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GraveEntry {
    pub oid: i64,    // Object ID
    pub gtype: i32,  // Type (0=card, 1=note, 2=deck)
    pub usn: i32,    // Update sequence number
}

/// Tags table entry
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TagEntry {
    pub tag: String,         // Tag name
    pub usn: i32,            // Update sequence number
    pub collapsed: i32,      // Whether tag is collapsed in UI (0 or 1)
    pub config: Option<Vec<u8>>, // Optional config blob
}

pub struct Package {
    pub decks: Vec<Deck>,
    media_files: Vec<MediaFile>,
    configs: Vec<ConfigEntry>,
    deck_configs: Vec<DeckConfigEntry>,
    deck_infos: Vec<DeckInfoEntry>,
    notetypes: Vec<NotetypeEntry>,
    field_entries: Vec<FieldEntry>,
    template_entries: Vec<TemplateEntry>,
    graves: Vec<GraveEntry>,
    tags: Vec<TagEntry>,
    // Custom col table overrides for preserving original Anki data
    col_crt: Option<i64>,
    col_ver: Option<i64>,
    col_scm: Option<i64>,
    col_usn: Option<i32>,
    col_ls: Option<i64>,
    col_conf: Option<String>,
    col_models: Option<String>,
    col_decks: Option<String>,
    col_dconf: Option<String>,
}

impl Package {
    /// Create a new package with `decks` and `media_files`
    ///
    /// Returns `Err` if `media_files` are invalid
    pub fn new(decks: Vec<Deck>, media_files: Vec<String>) -> Result<Self, Error> {
        let media_files = media_files
            .iter()
            .map(|s| PathBuf::from_str(s.as_str()).map(|p| MediaFile::Path(p)))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            decks,
            media_files,
            configs: Vec::new(),
            deck_configs: Vec::new(),
            deck_infos: Vec::new(),
            notetypes: Vec::new(),
            field_entries: Vec::new(),
            template_entries: Vec::new(),
            graves: Vec::new(),
            tags: Vec::new(),
            col_crt: None,
            col_ver: None,
            col_scm: None,
            col_usn: None,
            col_ls: None,
            col_conf: None,
            col_models: None,
            col_decks: None,
            col_dconf: None,
        })
    }

    /// Adds a configuration entry to the package.
    pub fn add_config_entry(&mut self, entry: ConfigEntry) {
        self.configs.push(entry);
    }

    /// Adds a deck configuration entry to the package.
    pub fn add_deck_config_entry(&mut self, entry: DeckConfigEntry) {
        self.deck_configs.push(entry);
    }

    /// Sets custom col table data to preserve original Anki metadata during export
    pub fn set_col_data(
        &mut self,
        crt: Option<i64>,
        ver: Option<i64>,
        scm: Option<i64>,
        usn: Option<i32>,
        ls: Option<i64>,
        conf: Option<String>,
        models: Option<String>,
        decks: Option<String>,
        dconf: Option<String>,
    ) {
        self.col_crt = crt;
        self.col_ver = ver;
        self.col_scm = scm;
        self.col_usn = usn;
        self.col_ls = ls;
        self.col_conf = conf;
        self.col_models = models;
        self.col_decks = decks;
        self.col_dconf = dconf;
    }

    /// Adds a deck info entry to the package.
    pub fn add_deck_info_entry(&mut self, entry: DeckInfoEntry) {
        self.deck_infos.push(entry);
    }

    /// Adds a notetype entry to the package.
    pub fn add_notetype_entry(&mut self, entry: NotetypeEntry) {
        self.notetypes.push(entry);
    }

    /// Adds a field entry to the package.
    pub fn add_field_entry(&mut self, entry: FieldEntry) {
        self.field_entries.push(entry);
    }

    /// Adds a template entry to the package.
    pub fn add_template_entry(&mut self, entry: TemplateEntry) {
        self.template_entries.push(entry);
    }

    /// Adds a grave entry (deleted item tombstone) to the package.
    pub fn add_grave_entry(&mut self, entry: GraveEntry) {
        self.graves.push(entry);
    }

    /// Adds a tag entry to the package.
    pub fn add_tag_entry(&mut self, entry: TagEntry) {
        self.tags.push(entry);
    }

    /// Create a new package with `decks` and `media_files`,
    /// where `media_files` can be bytes from memory or a path on the filesystem
    /// 
    /// Returns `Err` if `media_files` are invalid
    pub fn new_from_memory(decks: Vec<Deck>, media_files: Vec<MediaFile>) -> Result<Self, Error> {
        Ok(Self {
            decks,
            media_files,
            configs: Vec::new(),
            deck_configs: Vec::new(),
            deck_infos: Vec::new(),
            notetypes: Vec::new(),
            field_entries: Vec::new(),
            template_entries: Vec::new(),
            graves: Vec::new(),
            tags: Vec::new(),
            col_crt: None,
            col_ver: None,
            col_scm: None,
            col_usn: None,
            col_ls: None,
            col_conf: None,
            col_models: None,
            col_decks: None,
            col_dconf: None,
        })
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
            .collect::<HashMap<usize, &MediaFile>>();
        let media_map = media_file_idx_to_path
            .clone()
            .into_iter()
            .map(|(id, media_file)| {
                (
                    id.to_string(),
                    match media_file {
                        MediaFile::Path(path) => path.file_name()
                            .expect("Should always have a filename")
                            .to_str()
                            .expect("should always have string"),
                        MediaFile::Bytes(_, name) => name,
                    },
                )
            })
            .collect::<HashMap<String, &str>>();
        let media_json = serde_json::to_string(&media_map).map_err(json_error)?;
        outzip
            .start_file("media", FileOptions::default())
            .map_err(zip_error)?;
        outzip.write_all(media_json.as_bytes())?;

        for (idx, &media_file) in &media_file_idx_to_path {
            outzip
                .start_file(idx.to_string(), FileOptions::default())
                .map_err(zip_error)?;
            outzip.write_all(&match media_file {
                MediaFile::Path(path) => read_file_bytes(path)?,
                MediaFile::Bytes(bytes, _) => bytes.clone(),
            })?;
        }
        outzip.finish().map_err(zip_error)?;
        Ok(())
    }

    fn write_schema_and_col_table(&self, transaction: &Transaction, timestamp_sec: f64) -> Result<(), Error> {
        // Determine version early to use for conditional schema creation
        let ver: i64 = self.col_ver.unwrap_or(18);

        // Use version-appropriate schema
        if ver < 12 {
            // Anki 2.0 (version 11 and below) - minimal tables only
            transaction.execute_batch(APKG_SCHEMA_V11).map_err(database_error)?;
        } else {
            // Anki 2.1 (version 12+) - includes all modern tables
            transaction.execute_batch(APKG_SCHEMA).map_err(database_error)?;
        }

        // Create graves table with version-appropriate schema
        if ver >= 18 {
            // Anki 2.1 v18+ schema: includes PRIMARY KEY
            transaction.execute(
                "CREATE TABLE graves (
                    oid             integer not null,
                    type            integer not null,
                    usn             integer not null,
                    PRIMARY KEY (oid, type)
                )",
                [],
            ).map_err(database_error)?;
        } else {
            // Anki 2.0 v11 schema: no PRIMARY KEY
            transaction.execute(
                "CREATE TABLE graves (
                    usn             integer not null,
                    oid             integer not null,
                    type            integer not null
                )",
                [],
            ).map_err(database_error)?;
        }

        // Populate graves table with deleted items tombstones
        for grave in &self.graves {
            if ver >= 18 {
                transaction.execute(
                    "INSERT INTO graves (oid, type, usn) VALUES (?, ?, ?)",
                    params![grave.oid, grave.gtype, grave.usn],
                ).map_err(database_error)?;
            } else {
                // Anki 2.0 schema has different column order
                transaction.execute(
                    "INSERT INTO graves (usn, oid, type) VALUES (?, ?, ?)",
                    params![grave.usn, grave.oid, grave.gtype],
                ).map_err(database_error)?;
            }
        }

        // Populate tags table (only for version 12+ where tags table exists in schema)
        // For version 11 and below, tags are stored in the 'col' table's 'tags' column
        if ver >= 12 {
            for tag_entry in &self.tags {
                transaction.execute(
                    "INSERT INTO tags (tag, usn, collapsed, config) VALUES (?, ?, ?, ?)",
                    params![
                        tag_entry.tag,
                        tag_entry.usn,
                        tag_entry.collapsed,
                        tag_entry.config.as_ref().map(|v| v.as_slice())
                    ],
                ).map_err(database_error)?;
            }
        }

        // Initialize dconf_map_for_col before version check (needed for col table later)
        // In 'col' table, 'dconf' column is a JSON map of deck configs.
        let mut dconf_map_for_col: HashMap<String, serde_json::Value> = HashMap::new();

        // First, populate default dconf
        let default_dconf_json = "{\"1\": {\"autoplay\": true, \"id\": 1, \"lapse\": {\"delays\": [10], \"leechAction\": 0, \"leechFails\": 8, \"minInt\": 1, \"mult\": 0}, \"maxTaken\": 60, \"mod\": 0, \"name\": \"Default\", \"new\": {\"bury\": true, \"delays\": [1, 10], \"initialFactor\": 2500, \"ints\": [1, 4, 7], \"order\": 1, \"perDay\": 20, \"separate\": true}, \"replayq\": true, \"rev\": {\"bury\": true, \"ease4\": 1.3, \"fuzz\": 0.05, \"ivlFct\": 1, \"maxIvl\": 36500, \"minSpace\": 1, \"perDay\": 100}, \"timer\": 0, \"usn\": 0}}";

        // Try to parse default dconf string to initialize map
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(default_dconf_json) {
            if let Some(obj) = val.as_object() {
                for (k, v) in obj {
                    dconf_map_for_col.insert(k.clone(), v.clone());
                }
            }
        }

        // Only create and populate v12+ tables for Anki 2.1 (version 12+)
        if ver >= 12 {
            // Write config table entries
            // NOTE: In new Anki schema, 'config' table is key-value. 'conf' column in 'col' is global config JSON.
            // We populate the 'config' table if entries are provided, which some add-ons might use.
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
        // But newer Anki also uses 'deck_config' table. We write both for compatibility.
        for deck_config_entry in &self.deck_configs {
            // Write to deck_config table
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
                
            // Also add to dconf map for 'col' table
            if let Ok(json_val) = serde_json::from_slice::<serde_json::Value>(&deck_config_entry.config_blob) {
                dconf_map_for_col.insert(deck_config_entry.id.to_string(), json_val);
            }
        }

        // Create decks table (if it doesn't exist from APKG_SCHEMA - it usually doesn't define it explicitly)
        transaction.execute(
            "CREATE TABLE IF NOT EXISTS decks (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                mtime_secs INTEGER NOT NULL,
                usn INTEGER NOT NULL,
                common BLOB NOT NULL,
                kind BLOB NOT NULL
            )",
            [],
        ).map_err(database_error)?; // Ensure this uses map_err(database_error)

        // Insert deck_info entries
        for deck_info_entry in &self.deck_infos {
            transaction.execute(
                "INSERT INTO decks (id, name, mtime_secs, usn, common, kind) VALUES (?, ?, ?, ?, ?, ?)",
                params![
                    deck_info_entry.id,
                    deck_info_entry.name,
                    deck_info_entry.mtime_secs,
                    deck_info_entry.usn,
                    deck_info_entry.common,
                    deck_info_entry.kind
                ],
            ).map_err(database_error)?; // Ensure this uses map_err(database_error)
        }

            // Create notetypes table (Anki schema)
            transaction.execute(
                "CREATE TABLE IF NOT EXISTS notetypes (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    mtime_secs INTEGER NOT NULL,
                    usn INTEGER NOT NULL,
                    config BLOB NOT NULL
                )",
                [],
            ).map_err(database_error)?;

            // Insert notetype entries
            for notetype_entry in &self.notetypes {
                transaction.execute(
                    "INSERT INTO notetypes (id, name, mtime_secs, usn, config) VALUES (?, ?, ?, ?, ?)",
                    params![
                        notetype_entry.id,
                        notetype_entry.name,
                        notetype_entry.mtime_secs,
                        notetype_entry.usn,
                        notetype_entry.config,
                    ],
                ).map_err(database_error)?;
            }
            info!("Wrote {} entries to notetypes table.", self.notetypes.len());

            // Create fields table and insert data
            transaction.execute_batch(APKG_SCHEMA_FIELDS).map_err(database_error)?;
            let mut stmt_fields = transaction.prepare("INSERT INTO fields (ntid, ord, name, config) VALUES (?, ?, ?, ?)").map_err(database_error)?;
            for entry in &self.field_entries {
                stmt_fields.execute(params![entry.ntid, entry.ord, entry.name, entry.config]).map_err(database_error)?;
            }
            info!("Wrote {} entries to fields table.", self.field_entries.len());

            // Create templates table and insert data (CREATE TABLE is in APKG_SCHEMA)
            let mut stmt_templates = transaction.prepare("INSERT INTO templates (ntid, ord, name, mtime_secs, usn, config) VALUES (?, ?, ?, ?, ?, ?)").map_err(database_error)?;
            for entry in &self.template_entries {
                stmt_templates.execute(params![entry.ntid, entry.ord, entry.name, entry.mtime_secs, entry.usn, entry.config]).map_err(database_error)?;
            }
            info!("Wrote {} entries to templates table.", self.template_entries.len());
        } // End of version >= 12 block

        // Use custom col data if provided, otherwise compute defaults
        let crt_val = self.col_crt.unwrap_or_else(|| timestamp_sec as i64);
        let mod_val = (timestamp_sec * 1000.0) as i64;
        let scm_val = self.col_scm.unwrap_or(mod_val);

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
        // Also include manually added notetypes in col.models map if possible?
        // genanki typically derives col.models from the notes present.
        // The `notetypes` table entries are separate but should technically match.

        // Note: ver was already determined at the start of this method for graves table schema

        // Use custom models JSON if provided, otherwise compute from decks
        let models_json_str = if let Some(ref custom_models) = self.col_models {
            custom_models.clone()
        } else if ver >= 16 {
            "{}".to_string()
        } else {
            serde_json::to_string(&models_map_for_col).map_err(json_error)?
        };

        let mut decks_map_for_col: HashMap<String, DeckDbEntry> = HashMap::new();
        for deck_item in &self.decks {
            decks_map_for_col.insert(deck_item.id.to_string(), deck_item.to_deck_db_entry());
        }

        if !decks_map_for_col.contains_key("1") {
            let default_deck = Deck::new(1, "Default", "");
            decks_map_for_col.insert("1".to_string(), default_deck.to_deck_db_entry());
        }

        // Use custom decks JSON if provided, otherwise compute from decks
        let decks_json_str = if let Some(ref custom_decks) = self.col_decks {
            custom_decks.clone()
        } else if ver >= 16 {
            "{}".to_string()
        } else {
            serde_json::to_string(&decks_map_for_col).map_err(json_error)?
        };
        
        let default_conf_json = "{\"activeDecks\": [1], \"addToCur\": true, \"collapseTime\": 1200, \"curDeck\": 1, \"curModel\": \"1607392319\", \"dueCounts\": true, \"estTimes\": true, \"newBury\": true, \"newSpread\": 0, \"nextPos\": 1, \"sortBackwards\": false, \"sortType\": \"noteFld\", \"timeLim\": 0}";

        // Use custom conf if provided, otherwise use config_entry or default
        let conf_val = if let Some(ref custom_conf) = self.col_conf {
            custom_conf.clone()
        } else if ver >= 16 {
             "{}".to_string()
        } else if let Some(conf_entry) = self.configs.iter().find(|c| c.key == "conf") {
             std::str::from_utf8(&conf_entry.val).unwrap_or(default_conf_json).to_string()
        } else {
             default_conf_json.to_string()
        };

        // Use custom dconf if provided, otherwise compute from deck configs
        let dconf_json_str = if let Some(ref custom_dconf) = self.col_dconf {
            custom_dconf.clone()
        } else if ver >= 16 {
            "{}".to_string()
        } else {
            serde_json::to_string(&dconf_map_for_col).map_err(json_error)?
        };

        // Use the tags entry if it exists in the package or a string field from col_tags
        // Note: We don't currently have a separate tags table entry structure, but we can check config
        let tags_val = if let Some(tags_entry) = self.configs.iter().find(|c| c.key == "tags") {
            std::str::from_utf8(&tags_entry.val).unwrap_or("{}")
        } else {
            "{}"
        };

        // Use custom usn if provided, otherwise default to -1 (needs upload)
        let usn_val = self.col_usn.unwrap_or(-1);

        // Use custom ls (last sync) if provided, otherwise default to 0 (never synced)
        let ls_val = self.col_ls.unwrap_or(0);

        transaction.execute(
            "INSERT INTO col (id, crt, mod, scm, ver, dty, usn, ls, conf, models, decks, dconf, tags) VALUES (NULL, ?, ?, ?, ?, 0, ?, ?, ?, ?, ?, ?, ?)",
            params![
                crt_val,
                mod_val,
                scm_val,
                ver,
                usn_val,
                ls_val,
                conf_val,
                models_json_str,
                decks_json_str,
                dconf_json_str,
                tags_val
            ],
        ).map_err(database_error)?;
        Ok(())
    }

    fn write_deck_content_data(&mut self, transaction: &Transaction, timestamp_sec: f64) -> Result<(), Error> {
        let mut id_gen = ((timestamp_sec * 1000.0) as usize)..;
        log::info!("Writing content for {} decks", self.decks.len());
        for deck in &mut self.decks {
            log::info!("Writing content for deck {}: {} notes", deck.id, deck.notes().len());
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
