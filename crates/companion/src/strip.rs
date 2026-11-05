//! Reads the addon's live strip: a row of coloured squares in the top-left
//! corner of the game window, each carrying three bytes. See the header of
//! `addon/WowTracker/Strip.lua` for the layout. This module only decodes
//! pixels; grabbing them from the screen is in `capture.rs`.

use serde::Deserialize;
use wowtracker_protocol::{JournalEntry, LiveFrame, Quest, TrailPoint};

pub const FRAME_BYTES: usize = 93;
pub const CELLS: usize = FRAME_BYTES / 3;
pub const CHUNK: usize = 58;
/// Physical pixels per square drawn by the addon.
pub const CELL: usize = 3;
/// Largest square size tried when the game's scaling is off.
pub const MAX_CELL: usize = 6;
const VERSION: u8 = 1;

/// One decoded strip frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frame {
    pub seq: u8,
    pub in_combat: bool,
    pub dead: bool,
    pub ghost: bool,
    pub resting: bool,
    pub has_position: bool,
    pub level: u32,
    pub xp: u64,
    pub xp_max: u64,
    pub rested: u64,
    pub map_id: u32,
    pub x: f64,
    pub y: f64,
    pub money: u64,
    pub t: i64,
    pub chunk: Option<Chunk>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub id: u8,
    pub index: u8,
    pub count: u8,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecodeError {
    TooShort,
    NoMarker,
    Version(u8),
    Checksum,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::TooShort => write!(f, "too few bytes"),
            DecodeError::NoMarker => write!(f, "no strip marker (is the addon's strip on and the corner visible?)"),
            DecodeError::Version(v) => write!(f, "strip format {v} is not supported (update the companion)"),
            DecodeError::Checksum => write!(f, "checksum mismatch (colours changed on the way: set gamma/brightness to default, no HDR or colour filters)"),
        }
    }
}

pub fn fletcher16(bytes: &[u8]) -> (u8, u8) {
    let (mut a, mut b) = (0u32, 0u32);
    for &x in bytes {
        a = (a + x as u32) % 255;
        b = (b + a) % 255;
    }
    (a as u8, b as u8)
}

fn be(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |acc, &x| (acc << 8) | x as u64)
}

pub fn decode(b: &[u8]) -> Result<Frame, DecodeError> {
    if b.len() < FRAME_BYTES {
        return Err(DecodeError::TooShort);
    }
    if b[0] != b'W' || b[1] != b'T' {
        return Err(DecodeError::NoMarker);
    }
    if b[2] != VERSION {
        return Err(DecodeError::Version(b[2]));
    }
    let (s1, s2) = fletcher16(&b[..FRAME_BYTES - 2]);
    if (s1, s2) != (b[FRAME_BYTES - 2], b[FRAME_BYTES - 1]) {
        return Err(DecodeError::Checksum);
    }
    let flags = b[4];
    let (id, index, count, len) = (b[29], b[30], b[31], b[32] as usize);
    let chunk = (id != 0 && count > 0 && len <= CHUNK && index < count)
        .then(|| Chunk { id, index, count, data: b[33..33 + len].to_vec() });
    Ok(Frame {
        seq: b[3],
        in_combat: flags & 1 != 0,
        dead: flags & 2 != 0,
        ghost: flags & 4 != 0,
        resting: flags & 8 != 0,
        has_position: flags & 16 != 0,
        level: b[5] as u32,
        xp: be(&b[6..9]),
        xp_max: be(&b[9..12]),
        rested: be(&b[12..15]),
        map_id: be(&b[15..17]) as u32,
        x: be(&b[17..19]) as f64 / 65535.0,
        y: be(&b[19..21]) as f64 / 65535.0,
        money: be(&b[21..25]),
        t: be(&b[25..29]) as i64,
        chunk,
    })
}

/// Reads the bytes of one strip from a 32-bit BGRA image (rows top-down,
/// `stride` bytes per row), taking the centre pixel of each `cell`-sized square.
pub fn sample(pixels: &[u8], stride: usize, width: usize, height: usize, cell: usize) -> Option<Vec<u8>> {
    if cell == 0 || CELLS * cell > width || cell > height {
        return None;
    }
    let row = cell / 2;
    let mut out = Vec::with_capacity(FRAME_BYTES);
    for i in 0..CELLS {
        let col = i * cell + cell / 2;
        let p = row * stride + col * 4;
        let px = pixels.get(p..p + 4)?;
        out.extend([px[2], px[1], px[0]]); // BGRA -> R, G, B
    }
    Some(out)
}

/// Finds the strip at the top-left of the image, trying the expected square
/// size first. Returns the frame and the square size it was found at.
pub fn read_image(pixels: &[u8], stride: usize, width: usize, height: usize) -> Result<(Frame, usize), DecodeError> {
    let mut first_err = None;
    for cell in std::iter::once(CELL).chain((1..=MAX_CELL).filter(|&c| c != CELL)) {
        let Some(bytes) = sample(pixels, stride, width, height, cell) else { continue };
        match decode(&bytes) {
            Ok(f) => return Ok((f, cell)),
            Err(e) => {
                // Prefer the error from the expected size; "no marker" says little.
                if first_err.is_none() || first_err == Some(DecodeError::NoMarker) {
                    first_err = Some(e);
                }
            }
        }
    }
    Err(first_err.unwrap_or(DecodeError::TooShort))
}

/// Puts chunked messages back together.
#[derive(Default)]
pub struct Assembler {
    id: u8,
    parts: Vec<Option<Vec<u8>>>,
}

impl Assembler {
    /// Returns a whole message once its last missing chunk arrives.
    pub fn push(&mut self, c: &Chunk) -> Option<Vec<u8>> {
        if c.id != self.id || self.parts.len() != c.count as usize {
            self.id = c.id;
            self.parts = vec![None; c.count as usize];
        }
        let slot = self.parts.get_mut(c.index as usize)?;
        if slot.is_some() {
            return None; // a repeat (each chunk is shown twice)
        }
        *slot = Some(c.data.clone());
        if self.parts.iter().all(|p| p.is_some()) {
            let msg = self.parts.iter().flatten().flatten().copied().collect();
            // Mark as delivered: keep the slots filled so repeats are ignored.
            return Some(msg);
        }
        None
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Identity {
    pub key: Option<String>,
    pub name: Option<String>,
    pub realm: Option<String>,
    pub guid: Option<String>,
    pub class: Option<String>,
    pub class_file: Option<String>,
    pub race: Option<String>,
    pub faction: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Zone {
    pub zone: String,
    pub subzone: String,
    pub instance: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Identity(Identity),
    Zone(Zone),
    Quests(Vec<Quest>),
    Event(JournalEntry),
}

pub fn parse_message(bytes: &[u8]) -> Result<Message, String> {
    let (&kind, json) = bytes.split_first().ok_or("empty message")?;
    let text = String::from_utf8_lossy(json);
    let err = |e: serde_json::Error| format!("message {}: {e}", kind as char);
    Ok(match kind {
        b'I' => Message::Identity(serde_json::from_str(&text).map_err(err)?),
        b'Z' => Message::Zone(serde_json::from_str(&text).map_err(err)?),
        b'Q' => Message::Quests(serde_json::from_str(&text).map_err(err)?),
        b'E' => Message::Event(serde_json::from_str(&text).map_err(err)?),
        other => return Err(format!("unknown message type {:?}", other as char)),
    })
}

/// Movement (fraction of the map) that adds a trail point.
const TRAIL_MIN_MOVE: f64 = 0.0015;
/// Seconds between trail points on the same map, at least.
const TRAIL_MIN_GAP: i64 = 2;
/// Most events and trail points kept while the backend cannot be reached.
const PENDING_MAX: usize = 2000;

/// Turns a stream of frames into [`LiveFrame`]s to send.
#[derive(Default)]
pub struct Reader {
    last_seq: Option<u8>,
    assembler: Assembler,
    identity: Identity,
    zone: Option<Zone>,
    quests: Option<Vec<Quest>>,
    events: Vec<JournalEntry>,
    trail: Vec<TrailPoint>,
    last_point: Option<TrailPoint>,
    frame: Option<Frame>,
    /// Frames and messages decoded so far (for status lines).
    pub frames: u64,
    pub messages: u64,
    pub bad_messages: u64,
}

impl Reader {
    /// Takes in one decoded frame. Returns false for a repeat of the last one.
    pub fn feed(&mut self, f: Frame) -> bool {
        if self.last_seq == Some(f.seq) && self.frame.as_ref().is_some_and(|l| l.t == f.t) {
            return false;
        }
        self.last_seq = Some(f.seq);
        self.frames += 1;
        if let Some(c) = &f.chunk {
            if let Some(bytes) = self.assembler.push(c) {
                match parse_message(&bytes) {
                    Ok(m) => {
                        self.messages += 1;
                        self.apply(m);
                    }
                    Err(_) => self.bad_messages += 1,
                }
            }
        }
        if f.has_position && f.map_id != 0 {
            let p = TrailPoint { t: f.t, map_id: f.map_id, x: round4(f.x), y: round4(f.y) };
            let keep = match self.last_point {
                None => true,
                Some(l) => {
                    let moved = (l.x - p.x).abs() >= TRAIL_MIN_MOVE || (l.y - p.y).abs() >= TRAIL_MIN_MOVE;
                    l.map_id != p.map_id || (moved && p.t - l.t >= TRAIL_MIN_GAP)
                }
            };
            if keep {
                self.trail.push(p);
                self.last_point = Some(p);
                cap(&mut self.trail);
            }
        }
        self.frame = Some(f);
        true
    }

    fn apply(&mut self, m: Message) {
        match m {
            Message::Identity(i) => self.identity = i,
            Message::Zone(z) => self.zone = Some(z),
            Message::Quests(q) => self.quests = Some(q),
            Message::Event(e) => {
                self.events.push(e);
                cap(&mut self.events);
            }
        }
    }

    /// The state to send now: the latest frame plus everything collected
    /// since the last call. None before the first frame.
    pub fn take(&mut self) -> Option<LiveFrame> {
        let f = self.frame.as_ref()?;
        let i = &self.identity;
        let z = self.zone.clone().unwrap_or_default();
        let mut trail = std::mem::take(&mut self.trail);
        // The current position is sent as x/y, not again in the trail.
        if trail.last().is_some_and(|p| p.t == f.t) {
            trail.pop();
        }
        Some(LiveFrame {
            t: f.t,
            key: i.key.clone(),
            guid: i.guid.clone(),
            name: i.name.clone(),
            realm: i.realm.clone(),
            class: i.class.clone(),
            class_file: i.class_file.clone(),
            race: i.race.clone(),
            faction: i.faction.clone(),
            level: f.level,
            xp: f.xp,
            xp_max: f.xp_max,
            rested: f.rested,
            money: f.money,
            map_id: f.map_id,
            x: round4(f.x),
            y: round4(f.y),
            has_position: f.has_position,
            in_combat: f.in_combat,
            dead: f.dead,
            ghost: f.ghost,
            resting: f.resting,
            zone: self.zone.as_ref().map(|_| z.zone.clone()).filter(|s| !s.is_empty()),
            subzone: self.zone.as_ref().map(|_| z.subzone.clone()),
            instance: z.instance.clone(),
            quests: self.quests.take(),
            events: std::mem::take(&mut self.events),
            trail,
        })
    }

    /// Puts back what a failed send carried, so it goes with the next one.
    pub fn restore(&mut self, mut sent: LiveFrame) {
        sent.events.append(&mut self.events);
        self.events = sent.events;
        sent.trail.append(&mut self.trail);
        self.trail = sent.trail;
        cap(&mut self.events);
        cap(&mut self.trail);
        if self.quests.is_none() {
            self.quests = sent.quests;
        }
    }
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

fn cap<T>(v: &mut Vec<T>) {
    if v.len() > PENDING_MAX {
        v.drain(..v.len() - PENDING_MAX);
    }
}

/// Builds frame bytes the way the addon does. For tests (here and in the
/// addon harness, which checks the Lua side against this).
pub fn encode(f: &Frame) -> Vec<u8> {
    let mut b = vec![b'W', b'T', VERSION, f.seq];
    let flags = f.in_combat as u8 | (f.dead as u8) << 1 | (f.ghost as u8) << 2 | (f.resting as u8) << 3 | (f.has_position as u8) << 4;
    b.push(flags);
    b.push(f.level.min(255) as u8);
    let mut put = |n: u64, size: usize| {
        for i in (0..size).rev() {
            b.push((n >> (8 * i)) as u8);
        }
    };
    put(f.xp, 3);
    put(f.xp_max, 3);
    put(f.rested, 3);
    put(f.map_id as u64, 2);
    put((f.x * 65535.0).round() as u64, 2);
    put((f.y * 65535.0).round() as u64, 2);
    put(f.money, 4);
    put(f.t as u64, 4);
    match &f.chunk {
        Some(c) => {
            b.extend([c.id, c.index, c.count, c.data.len() as u8]);
            b.extend(&c.data);
            b.resize(33 + CHUNK, 0);
        }
        None => b.resize(33 + CHUNK, 0),
    }
    let (s1, s2) = fletcher16(&b);
    b.extend([s1, s2]);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(seq: u8, t: i64) -> Frame {
        Frame {
            seq,
            in_combat: true,
            resting: false,
            has_position: true,
            level: 12,
            xp: 4321,
            xp_max: 8800,
            rested: 1200,
            map_id: 1429,
            x: 0.4123,
            y: 0.6512,
            money: 123_456,
            t,
            ..Default::default()
        }
    }

    /// Paints bytes as squares into a BGRA image, like the game would.
    fn paint(bytes: &[u8], cell: usize, width: usize, height: usize) -> Vec<u8> {
        let mut img = vec![40u8; width * height * 4];
        for (i, rgb) in bytes.chunks(3).enumerate() {
            for y in 0..cell {
                for x in 0..cell {
                    let p = (y * width + i * cell + x) * 4;
                    img[p..p + 4].copy_from_slice(&[rgb[2], rgb[1], rgb[0], 255]);
                }
            }
        }
        img
    }

    #[test]
    fn round_trip() {
        let f = frame(7, 1_793_900_000);
        let b = encode(&f);
        assert_eq!(b.len(), FRAME_BYTES);
        let d = decode(&b).unwrap();
        assert_eq!((d.level, d.xp, d.xp_max, d.rested, d.map_id, d.money, d.t), (12, 4321, 8800, 1200, 1429, 123_456, 1_793_900_000));
        assert!((d.x - 0.4123).abs() < 1e-4 && (d.y - 0.6512).abs() < 1e-4);
        assert!(d.in_combat && d.has_position && !d.dead);
    }

    #[test]
    fn rejects_damaged_frames() {
        let mut b = encode(&frame(1, 100));
        b[10] ^= 1;
        assert_eq!(decode(&b), Err(DecodeError::Checksum));
        b[0] = 0;
        assert_eq!(decode(&b), Err(DecodeError::NoMarker));
        assert_eq!(decode(&b[..10]), Err(DecodeError::TooShort));
    }

    #[test]
    fn reads_from_pixels_at_any_cell_size() {
        let b = encode(&frame(3, 500));
        for cell in [CELL, 2, 4] {
            let (w, h) = (CELLS * MAX_CELL + 10, MAX_CELL + 2);
            let img = paint(&b, cell, w, h);
            let (f, found) = read_image(&img, w * 4, w, h).unwrap();
            assert_eq!((f.t, found), (500, cell));
        }
        let blank = vec![0u8; 400 * 10 * 4];
        assert!(read_image(&blank, 1600, 400, 10).is_err());
    }

    #[test]
    fn messages_across_chunks() {
        let text = br#"E{"kind":"kill","t":100,"title":"Kobold Vermin","xp":45}"#.repeat(1);
        let mut long = b"Q[".to_vec();
        for i in 0..5 {
            long.extend(format!(r#"{{"id":{i},"title":"Quest number {i} with a long name","complete":false,"objectives":[]}},"#).bytes());
        }
        long.pop();
        long.push(b']');
        let mut r = Reader::default();
        let mut seq = 0u8;
        let mut t = 1000;
        for (id, msg) in [(1u8, text.as_slice()), (2, long.as_slice())] {
            let parts: Vec<&[u8]> = msg.chunks(CHUNK).collect();
            for (i, part) in parts.iter().enumerate() {
                for _ in 0..2 {
                    seq = seq.wrapping_add(1);
                    t += 1;
                    let mut f = frame(seq, t);
                    f.chunk = Some(Chunk { id, index: i as u8, count: parts.len() as u8, data: part.to_vec() });
                    assert!(r.feed(decode(&encode(&f)).unwrap()));
                }
            }
        }
        assert_eq!(r.messages, 2);
        let live = r.take().unwrap();
        assert_eq!(live.events.len(), 1);
        assert_eq!(live.events[0].kind, "kill");
        assert_eq!(live.quests.as_ref().map(|q| q.len()), Some(5));
        assert_eq!(live.level, 12);
        // Taken: the next send carries none of it again.
        let again = r.take().unwrap();
        assert!(again.events.is_empty() && again.quests.is_none());
    }

    #[test]
    fn repeats_and_trail() {
        let mut r = Reader::default();
        assert!(r.feed(frame(1, 100)));
        assert!(!r.feed(frame(1, 100)), "same frame read twice");
        let mut f = frame(2, 102);
        f.x += 0.01;
        assert!(r.feed(f.clone()));
        let mut f = frame(3, 103);
        f.x += 0.0101; // barely moved
        assert!(r.feed(f));
        let live = r.take().unwrap();
        assert_eq!(live.trail.len(), 2, "first point and the move; the current position is x/y");
        assert_eq!(live.t, 103);

        // A failed send is put back in front of newer data.
        let mut r2 = Reader::default();
        r2.feed(frame(1, 100));
        let mut e = r2.take().unwrap();
        e.events.push(JournalEntry { t: 99, kind: "death".into(), ..Default::default() });
        r2.restore(e);
        assert_eq!(r2.take().unwrap().events.len(), 1);
    }

    #[test]
    fn bad_message_is_counted_not_fatal() {
        let mut r = Reader::default();
        let mut f = frame(1, 100);
        f.chunk = Some(Chunk { id: 9, index: 0, count: 1, data: b"Q{not json".to_vec() });
        r.feed(f);
        assert_eq!((r.messages, r.bad_messages), (0, 1));
        assert!(parse_message(b"X{}").is_err());
    }
}
