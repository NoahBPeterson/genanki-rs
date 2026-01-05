use rusqlite::{params, Transaction};
use std::ops::RangeFrom;

use crate::{error::database_error, Error};

/// Represents a single review log entry from Anki's revlog table
#[derive(Clone, Debug)]
pub struct RevlogEntry {
    pub id: i64,           // Timestamp when review occurred
    pub ease: i32,         // Button pressed (1=again, 2=hard, 3=good, 4=easy)
    pub ivl: i32,          // Interval after review
    pub last_ivl: i32,     // Interval before review
    pub factor: i32,       // Ease factor after review
    pub time: i32,         // Time taken to answer (milliseconds)
    pub review_type: i32,  // Type of review (0=learn, 1=review, 2=relearn, 3=cram)
    pub usn: i32,          // Update sequence number
}

#[derive(Clone)]
pub struct Card {
    pub ord: i64,
    pub suspend: bool,
    // Optional review data fields - if None, defaults to new card values
    pub reps: Option<i32>,      // Number of reviews
    pub lapses: Option<i32>,    // Number of lapses/failures
    pub ivl: Option<i32>,       // Interval in days
    pub due: Option<i64>,       // Due date (Anki format)
    pub factor: Option<i32>,    // Ease factor (e.g., 2500 = 2.5)
    pub card_type: Option<i32>, // Card type (0=new, 1=learning, 2=review, 3=relearning)
    pub queue: Option<i32>,     // Queue type
    pub left: Option<i32>,      // Steps remaining
    pub review_history: Vec<RevlogEntry>, // Review history for this card
    pub data: Option<String>,  // Added data field for FSRS JSON etc.
    pub custom_card_id: Option<i64>, // Custom card ID to use instead of generated one
    pub usn: i32,              // Update sequence number (default: -1)
}

impl Card {
    pub fn new(ord: i64, suspend: bool) -> Self {
        Self {
            ord,
            suspend,
            reps: None,
            lapses: None,
            ivl: None,
            due: None,
            factor: None,
            card_type: None,
            queue: None,
            left: None,
            review_history: Vec::new(),
            data: None, // Initialize new field
            custom_card_id: None,
            usn: -1,
        }
    }

    /// Create a card with review data for cards that have learning history
    pub fn new_with_review_data(
        ord: i64,
        suspend: bool,
        reps: i32,
        lapses: i32,
        ivl: i32,
        due: i64,
        factor: i32,
        card_type: i32,
        queue: i32,
        left: i32,
    ) -> Self {
        Self {
            ord,
            suspend,
            reps: Some(reps),
            lapses: Some(lapses),
            ivl: Some(ivl),
            due: Some(due),
            factor: Some(factor),
            card_type: Some(card_type),
            queue: Some(queue),
            left: Some(left),
            review_history: Vec::new(),
            data: None, // Initialize, to be set by a setter or new constructor variant if needed
            custom_card_id: None,
            usn: -1,
        }
    }

    /// Create a card with review data and review history
    pub fn new_with_review_history(
        ord: i64,
        suspend: bool,
        reps: i32,
        lapses: i32,
        ivl: i32,
        due: i64,
        factor: i32,
        card_type: i32,
        queue: i32,
        left: i32,
        review_history: Vec<RevlogEntry>,
        data: Option<String>, // Added data parameter
    ) -> Self {
        Self {
            ord,
            suspend,
            reps: Some(reps),
            lapses: Some(lapses),
            ivl: Some(ivl),
            due: Some(due),
            factor: Some(factor),
            card_type: Some(card_type),
            queue: Some(queue),
            left: Some(left),
            review_history,
            data, // Assign from parameter
            custom_card_id: None,
            usn: -1,
        }
    }

    /// Sets the USN (update sequence number) for this card
    ///
    /// By default, USN is -1 (indicating local changes not synced).
    /// Use this method to preserve USN values from imported Anki decks.
    pub fn set_usn(mut self, usn: i32) -> Self {
        self.usn = usn;
        self
    }

    #[allow(dead_code)]
    pub fn ord(&self) -> i64 {
        self.ord
    }

    pub fn write_to_db(
        &self,
        transaction: &Transaction,
        timestamp: f64,
        deck_id: i64,
        note_id: usize,
        id_gen: &mut RangeFrom<usize>,
    ) -> Result<(), Error> {
        let queue = if self.suspend { 
            -1 
        } else { 
            self.queue.unwrap_or(0) 
        };
        
        // Use custom card ID if provided, otherwise generate one
        let card_id = if let Some(custom_id) = self.custom_card_id {
            custom_id as usize
        } else {
            id_gen.next().unwrap()
        };
        
        transaction
            .execute(
                "INSERT INTO cards VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?);",
                params![
                    card_id,                             // id (idx 0)
                    note_id,                             // nid (idx 1)
                    deck_id,                             // did (idx 2)
                    self.ord,                            // ord (idx 3)
                    timestamp as i64,                    // mod (idx 4)
                    self.usn,                            // usn (idx 5)
                    self.card_type.unwrap_or(0),         // type (idx 6)
                    queue,                               // queue (idx 7)
                    self.due.unwrap_or(0),               // due (idx 8)
                    self.ivl.unwrap_or(0),               // ivl (idx 9)
                    self.factor.unwrap_or(0),            // factor (idx 10)
                    self.reps.unwrap_or(0),              // reps (idx 11)
                    self.lapses.unwrap_or(0),            // lapses (idx 12)
                    self.left.unwrap_or(0),              // left (idx 13)
                    0,                                   // odue (idx 14)
                    0,                                   // odid (idx 15)
                    0,                                   // flags (idx 16)
                    self.data.as_deref().unwrap_or(""),    // data (idx 17)
                ],
            )
            .map_err(database_error)?;

        // Write review history to revlog table
        for revlog_entry in &self.review_history {
            transaction
                .execute(
                    "INSERT INTO revlog VALUES(?,?,?,?,?,?,?,?,?);",
                    params![
                        revlog_entry.id,                 // id (timestamp)
                        card_id,                         // cid (card id)
                        revlog_entry.usn,                // usn
                        revlog_entry.ease,               // ease
                        revlog_entry.ivl,                // ivl
                        revlog_entry.last_ivl,           // lastIvl
                        revlog_entry.factor,             // factor
                        revlog_entry.time,               // time
                        revlog_entry.review_type,        // type
                    ],
                )
                .map_err(database_error)?;
        }
        
        Ok(())
    }
}
