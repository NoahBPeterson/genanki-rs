pub const APKG_SCHEMA: &str = r#"
CREATE TABLE col (
    id              integer primary key,
    crt             integer not null,
    mod             integer not null,
    scm             integer not null,
    ver             integer not null,
    dty             integer not null,
    usn             integer not null,
    ls              integer not null,
    conf            text not null,
    models          text not null,
    decks           text not null,
    dconf           text not null,
    tags            text not null
);
CREATE TABLE notes (
    id              integer primary key,   /* 0 */
    guid            text not null,         /* 1 */
    mid             integer not null,      /* 2 */
    mod             integer not null,      /* 3 */
    usn             integer not null,      /* 4 */
    tags            text not null,         /* 5 */
    flds            text not null,         /* 6 */
    sfld            text not null,         /* 7 */
    csum            integer not null,      /* 8 */
    flags           integer not null,      /* 9 */
    data            text not null          /* 10 */
);
CREATE TABLE cards (
    id              integer primary key,   /* 0 */
    nid             integer not null,      /* 1 */
    did             integer not null,      /* 2 */
    ord             integer not null,      /* 3 */
    mod             integer not null,      /* 4 */
    usn             integer not null,      /* 5 */
    type            integer not null,      /* 6 */
    queue           integer not null,      /* 7 */
    due             integer not null,      /* 8 */
    ivl             integer not null,      /* 9 */
    factor          integer not null,      /* 10 */
    reps            integer not null,      /* 11 */
    lapses          integer not null,      /* 12 */
    left            integer not null,      /* 13 */
    odue            integer not null,      /* 14 */
    odid            integer not null,      /* 15 */
    flags           integer not null,      /* 16 */
    data            text not null          /* 17 */
);
CREATE TABLE revlog (
    id              integer primary key,
    cid             integer not null,
    usn             integer not null,
    ease            integer not null,
    ivl             integer not null,
    lastIvl         integer not null,
    factor          integer not null,
    time            integer not null,
    type            integer not null
);
CREATE TABLE graves (
    usn             integer not null,
    oid             integer not null,
    type            integer not null
);
CREATE TABLE decks (
    id              integer primary key not null,
    name            text not null,
    mtime_secs      integer not null,
    usn             integer not null,
    common          text not null, 
    kind            text not null
);
CREATE TABLE deck_config (
    id              integer primary key not null,
    name            text not null,
    mtime_secs      integer not null,
    usn             integer not null,
    config          text not null
);
CREATE TABLE notetypes (
    id              integer primary key not null,
    name            text not null,
    mtime_secs      integer not null,
    usn             integer not null,
    config          text not null
);
CREATE TABLE templates (
    ntid            integer not null,
    ord             integer not null,
    name            text not null,
    mtime_secs      integer not null,
    usn             integer not null,
    config          text not null,
    PRIMARY KEY (ntid, ord)
);
CREATE TABLE fields (
    ntid            integer not null,
    ord             integer not null,
    name            text not null,
    config          text not null,
    PRIMARY KEY (ntid, ord)
);
CREATE TABLE config (
    key             text primary key not null,
    usn             integer not null,
    mtime_secs      integer not null,
    val             blob not null
);
CREATE INDEX ix_notes_usn on notes (usn);
CREATE INDEX ix_cards_usn on cards (usn);
CREATE INDEX ix_revlog_usn on revlog (usn);
CREATE INDEX ix_cards_nid on cards (nid);
CREATE INDEX ix_cards_sched on cards (did, queue, due);
CREATE INDEX ix_revlog_cid on revlog (cid);
CREATE INDEX ix_notes_csum on notes (csum);
CREATE INDEX ix_decks_usn ON decks (usn);
CREATE INDEX ix_deck_config_usn ON deck_config (usn);
CREATE INDEX ix_notetypes_usn ON notetypes (usn);
CREATE INDEX ix_templates_usn ON templates (usn);
CREATE INDEX ix_fields_ntid ON fields (ntid);
"#;

pub const APKG_SCHEMA_NOTETYPES: &str = r#"
CREATE TABLE IF NOT EXISTS notetypes (
    id              integer primary key not null,
    name            text not null,
    mtime_secs      integer not null,
    usn             integer not null,
    config          blob not null
);
CREATE INDEX IF NOT EXISTS ix_notetypes_usn ON notetypes (usn);
"#;

pub const APKG_SCHEMA_FIELDS: &str = r#"
CREATE TABLE IF NOT EXISTS fields (
    ntid            integer not null,
    ord             integer not null,
    name            text not null,
    config          blob not null,
    PRIMARY KEY (ntid, ord)
);
CREATE INDEX IF NOT EXISTS ix_fields_ntid ON fields (ntid);
"#;
