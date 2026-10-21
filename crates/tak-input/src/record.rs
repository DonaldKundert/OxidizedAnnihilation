//! Session recording: everything the input layer saw and did, frame by
//! frame, so a play session can come back for analysis and be replayed as a
//! ghost cursor over the match.
//!
//! A [`Recording`] holds the profile in use, free-form match metadata from
//! the client (map, players, seed, the sim command log) and one
//! [`RecFrame`] per client frame in which anything changed: the raw device
//! events, the cursor (screen and ground), the camera, the selection (when
//! it changed), the open wheel, the brush, the outputs (client and sim
//! commands) and feedback. Frames are timestamped with the adapter's clock
//! (ms) and the sim tick.

use serde::{Deserialize, Serialize};
use tak_commands::{SimCommand, UnitSet, WorldPos};
use tak_math::{Fx, Vec2};
use thiserror::Error;

use crate::binding::Profile;
use crate::event::EventKind;
use crate::mapper::{Feedback, Mapper, Output};
use crate::tuning::Tuning;
use crate::wheel::Wheel;
use crate::world::Camera;

/// Current file format version.
pub const RECORDING_VERSION: u32 = 1;

/// A recorded session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    /// [`RECORDING_VERSION`] when written.
    pub version: u32,
    /// Client metadata: map, players, seed, command log, build, ...
    #[serde(default)]
    pub meta: serde_json::Value,
    /// The profile at the start (later tuning changes are in the frames).
    pub profile: Profile,
    /// Frames in time order.
    pub frames: Vec<RecFrame>,
}

/// One client frame in which something changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecFrame {
    /// Adapter clock, ms.
    pub t: u32,
    /// Sim tick the frame's orders were stamped with.
    pub tick: u32,
    /// Raw device events fed to the mapper this frame (frame markers left
    /// out).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<EventKind>,
    /// Cursor, screen pixels.
    pub cursor: Vec2,
    /// Cursor on the ground.
    pub cursor_world: WorldPos,
    /// The stick (not the mouse) drives the cursor.
    pub stick: bool,
    /// Camera.
    pub camera: Camera,
    /// The new selection, when it changed this frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<UnitSet>,
    /// The open wheel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wheel: Option<Wheel>,
    /// The active brush, `(centre, radius)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<(WorldPos, Fx)>,
    /// Snap target, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snap: Option<tak_commands::UnitId>,
    /// What the mapper produced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<Output>,
    /// Sim orders as submitted, with the tick they execute on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub submitted: Vec<(u32, SimCommand)>,
    /// Feedback events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feedback: Vec<Feedback>,
    /// New tuning, when it changed this frame (overlay, hot reload).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tuning: Option<Tuning>,
}

/// Why a recording could not be read.
#[derive(Debug, Error)]
pub enum RecordingError {
    /// Not a recording.
    #[error("recording JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Written by a newer format.
    #[error("recording version {0} is newer than {RECORDING_VERSION}")]
    Version(u32),
}

/// What a ghost shows at one tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ghost {
    /// Cursor on the ground.
    pub cursor_world: WorldPos,
    /// Camera at that frame.
    pub camera: Camera,
    /// Selection in force.
    pub selection: UnitSet,
    /// Open wheel.
    pub wheel: Option<Wheel>,
    /// Active brush.
    pub brush: Option<(WorldPos, Fx)>,
    /// Stick cursor (else mouse).
    pub stick: bool,
}

/// Counts for a quick look at a session.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Summary {
    /// First to last frame, ms.
    pub duration_ms: u32,
    /// Frames stored.
    pub frames: u32,
    /// Device events.
    pub events: u32,
    /// Sim orders, by command type.
    pub orders: std::collections::BTreeMap<String, u32>,
    /// Selection changes.
    pub selections: u32,
    /// Snap feedback events.
    pub snaps: u32,
    /// Time with the stick driving the cursor, ms.
    pub stick_ms: u32,
    /// Mean stick-cursor speed while it moved, screen px/s.
    pub stick_speed_px_s: u32,
}

impl Recording {
    /// An empty recording.
    pub fn new(meta: serde_json::Value, profile: Profile) -> Recording {
        Recording {
            version: RECORDING_VERSION,
            meta,
            profile,
            frames: Vec::new(),
        }
    }

    /// Parse.
    pub fn from_json(s: &str) -> Result<Recording, RecordingError> {
        let r: Recording = serde_json::from_str(s)?;
        if r.version > RECORDING_VERSION {
            return Err(RecordingError::Version(r.version));
        }
        Ok(r)
    }

    /// Compact JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Index of the last frame at or before `tick`.
    fn index_at(&self, tick: u32) -> Option<usize> {
        let n = self.frames.partition_point(|f| f.tick <= tick);
        n.checked_sub(1)
    }

    /// The ghost at `tick`: the last frame at or before it, with the
    /// selection then in force.
    pub fn ghost_at(&self, tick: u32) -> Option<Ghost> {
        let i = self.index_at(tick)?;
        let f = &self.frames[i];
        let selection = self.frames[..=i]
            .iter()
            .rev()
            .find_map(|f| f.selection.clone())
            .unwrap_or_default();
        Some(Ghost {
            cursor_world: f.cursor_world,
            camera: f.camera,
            selection,
            wheel: f.wheel.clone(),
            brush: f.brush,
            stick: f.stick,
        })
    }

    /// Counts for a quick look.
    pub fn summary(&self) -> Summary {
        let mut s = Summary {
            frames: self.frames.len() as u32,
            ..Summary::default()
        };
        if let (Some(a), Some(b)) = (self.frames.first(), self.frames.last()) {
            s.duration_ms = b.t.saturating_sub(a.t);
        }
        let mut moved = 0i64;
        let mut moving_ms = 0u32;
        for w in self.frames.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let dt = b.t.saturating_sub(a.t);
            if b.stick {
                s.stick_ms += dt;
                let d = b.cursor.distance(a.cursor);
                if d > Fx::ZERO && dt > 0 && dt < 500 {
                    moved += i64::from(d.raw());
                    moving_ms += dt;
                }
            }
        }
        if moving_ms > 0 {
            s.stick_speed_px_s =
                (moved * 1000 / i64::from(moving_ms) / i64::from(Fx::ONE_RAW)) as u32;
        }
        for f in &self.frames {
            s.events += f.events.len() as u32;
            s.selections += u32::from(f.selection.is_some());
            s.snaps += f
                .feedback
                .iter()
                .filter(|x| matches!(x, Feedback::Snap))
                .count() as u32;
            for o in &f.outputs {
                if let Output::Sim { command, .. } = o {
                    let name = serde_json::to_value(command)
                        .ok()
                        .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
                        .unwrap_or_else(|| "?".into());
                    *s.orders.entry(name).or_default() += 1;
                }
            }
        }
        s
    }
}

/// What went in and out of the mapper during one client frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameIo<'a> {
    /// Device events fed to the mapper.
    pub events: &'a [EventKind],
    /// Its outputs.
    pub outputs: &'a [Output],
    /// Sim orders as submitted, with the tick they execute on.
    pub submitted: &'a [(u32, SimCommand)],
    /// Feedback drained this frame.
    pub feedback: &'a [Feedback],
}

/// Builds a [`Recording`] frame by frame, storing only frames in which
/// something changed.
#[derive(Clone, Debug)]
pub struct Recorder {
    rec: Recording,
    last_selection: UnitSet,
    last_tuning: Tuning,
}

impl Recorder {
    /// Start a recording for a mapper.
    pub fn new(meta: serde_json::Value, mapper: &Mapper) -> Recorder {
        Recorder {
            rec: Recording::new(meta, mapper.profile().clone()),
            last_selection: mapper.selection().clone(),
            last_tuning: *mapper.tuning(),
        }
    }

    /// Record one client frame, after the mapper handled `io.events`.
    pub fn frame(&mut self, mapper: &Mapper, t: u32, tick: u32, io: FrameIo<'_>) {
        let FrameIo {
            events,
            outputs,
            submitted,
            feedback,
        } = io;
        let events: Vec<EventKind> = events
            .iter()
            .filter(|e| !matches!(e, EventKind::Frame))
            .cloned()
            .collect();
        let selection = (mapper.selection() != &self.last_selection).then(|| {
            self.last_selection = mapper.selection().clone();
            self.last_selection.clone()
        });
        let tuning = (mapper.tuning() != &self.last_tuning).then(|| {
            self.last_tuning = *mapper.tuning();
            self.last_tuning
        });
        let f = RecFrame {
            t,
            tick,
            events,
            cursor: mapper.cursor_screen(),
            cursor_world: mapper.cursor_world(),
            stick: mapper.stick_cursor(),
            camera: *mapper.camera(),
            selection,
            wheel: mapper.wheel().cloned(),
            brush: mapper.brush(),
            snap: mapper.snap_target(),
            outputs: outputs.to_vec(),
            submitted: submitted.to_vec(),
            feedback: feedback.to_vec(),
            tuning,
        };
        if let Some(last) = self.rec.frames.last() {
            let same = f.events.is_empty()
                && f.selection.is_none()
                && f.outputs.is_empty()
                && f.submitted.is_empty()
                && f.feedback.is_empty()
                && f.tuning.is_none()
                && f.cursor == last.cursor
                && f.camera == last.camera
                && f.wheel == last.wheel
                && f.brush == last.brush;
            if same {
                return;
            }
        }
        self.rec.frames.push(f);
    }

    /// The recording so far.
    pub fn recording(&self) -> &Recording {
        &self.rec
    }

    /// Set the metadata (e.g. the final command log at exit).
    pub fn set_meta(&mut self, meta: serde_json::Value) {
        self.rec.meta = meta;
    }

    /// Finish.
    pub fn finish(self) -> Recording {
        self.rec
    }
}
