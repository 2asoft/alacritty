//! Live allocation measurement for Kitty graphics through the public fuzzing boundary.
//!
//! One case per process. Peak bytes are additional live `GlobalAlloc` requests relative to
//! caller-owned preallocated wire input and harness state. Visible snapshot pixels are reported
//! separately. This is not a timing measurement.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::graphics::GraphicsRenderSnapshot;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, GraphicsConfig, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

const FRAME_SIDE: u32 = 2048;
const FRAME_BYTES: usize = 16 * 1024 * 1024;
const RAW_CHUNK_BYTES: usize = 96 * 1024;
const ORDERING: Ordering = Ordering::SeqCst;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

fn account_alloc(size: usize) {
    let live = LIVE.fetch_add(size, ORDERING) + size;
    PEAK.fetch_max(live, ORDERING);
}

// SAFETY: Every operation forwards the original pointer and layout to `System` exactly once.
// Counters do not allocate and do not change ownership, size, or alignment.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            account_alloc(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            account_alloc(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), ORDERING);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, layout, new_size) };
        if !result.is_null() {
            LIVE.fetch_sub(layout.size(), ORDERING);
            account_alloc(new_size);
        }
        result
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Clone)]
struct RecordingListener {
    responses: Arc<Mutex<Vec<String>>>,
}

impl RecordingListener {
    fn new() -> Self {
        Self { responses: Arc::new(Mutex::new(Vec::new())) }
    }

    fn responses(&self) -> Vec<String> {
        self.responses.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }
}

impl EventListener for RecordingListener {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(response) = event {
            self.responses.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).push(response);
        }
    }
}

#[derive(Clone, Copy)]
enum Case {
    AnimationFrame,
    ComposeFrames,
    EditFrame,
    DirectRgb,
    DirectRgba,
}

#[derive(Clone, Copy)]
enum PixelKind {
    Rgba { fill: u8 },
    Rgb { fill: u8 },
    RgbaPixel([u8; 4]),
}

impl Case {
    fn parse(name: &str) -> Option<Self> {
        match name {
            "animation_frame" => Some(Self::AnimationFrame),
            "compose_frames" => Some(Self::ComposeFrames),
            "edit_frame" => Some(Self::EditFrame),
            "direct_rgb" => Some(Self::DirectRgb),
            "direct_rgba" => Some(Self::DirectRgba),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::AnimationFrame => "animation_frame",
            Self::ComposeFrames => "compose_frames",
            Self::EditFrame => "edit_frame",
            Self::DirectRgb => "direct_rgb",
            Self::DirectRgba => "direct_rgba",
        }
    }

    fn quota_bytes(self) -> usize {
        match self {
            Self::AnimationFrame | Self::ComposeFrames => 2 * FRAME_BYTES,
            Self::EditFrame | Self::DirectRgb | Self::DirectRgba => FRAME_BYTES,
        }
    }

    fn expected_ok(self) -> usize {
        match self {
            Self::AnimationFrame | Self::ComposeFrames => 3,
            Self::EditFrame => 2,
            Self::DirectRgb | Self::DirectRgba => 1,
        }
    }

    fn expected_pixels(self) -> PixelKind {
        match self {
            Self::AnimationFrame => PixelKind::Rgba { fill: 0x11 },
            Self::ComposeFrames => PixelKind::Rgba { fill: 0x33 },
            Self::EditFrame => PixelKind::RgbaPixel([44, 44, 44, 65]),
            Self::DirectRgb => PixelKind::Rgb { fill: 0x44 },
            Self::DirectRgba => PixelKind::Rgba { fill: 0x55 },
        }
    }
}

fn base64_len(raw_len: usize) -> usize {
    raw_len.div_ceil(3) * 4
}

fn push_apc(wire: &mut Vec<u8>, control: &str, payload: &[u8]) {
    wire.extend_from_slice(b"\x1b_G");
    wire.extend_from_slice(control.as_bytes());
    wire.push(b';');
    wire.extend_from_slice(payload);
    wire.extend_from_slice(b"\x1b\\");
}

fn push_direct_image(
    wire: &mut Vec<u8>,
    raw_chunk: &mut [u8],
    encoded_chunk: &mut [u8],
    control: &str,
    fill: u8,
    raw_len: usize,
) {
    raw_chunk.fill(fill);
    let mut remaining = raw_len;
    let mut first = true;
    while remaining != 0 {
        let chunk_len = remaining.min(RAW_CHUNK_BYTES);
        remaining -= chunk_len;
        let encoded_len = BASE64
            .encode_slice(&raw_chunk[..chunk_len], encoded_chunk)
            .expect("preallocated base64 chunk fits");
        let more = u8::from(remaining != 0);
        let header = if first {
            first = false;
            format!("{control},m={more}")
        } else {
            format!("m={more}")
        };
        push_apc(wire, &header, &encoded_chunk[..encoded_len]);
    }
}

const fn rgba_raw_len() -> usize {
    FRAME_SIDE as usize * FRAME_SIDE as usize * 4
}

fn rgb_raw_len() -> usize {
    usize::try_from(FRAME_SIDE).expect("frame side fits usize")
        * usize::try_from(FRAME_SIDE).expect("frame side fits usize")
        * 3
}

fn build_wire(case: Case) -> Vec<u8> {
    let mut raw_chunk = vec![0u8; RAW_CHUNK_BYTES];
    let mut encoded_chunk = vec![0u8; base64_len(RAW_CHUNK_BYTES)];
    let mut wire = Vec::new();
    let rgba = format!("a=T,i={{id}},f=32,s={FRAME_SIDE},v={FRAME_SIDE},c=1,r=1,C=1");
    match case {
        Case::AnimationFrame => {
            push_direct_image(
                &mut wire,
                &mut raw_chunk,
                &mut encoded_chunk,
                &rgba.replace("{id}", "1"),
                0x11,
                rgba_raw_len(),
            );
            push_direct_image(
                &mut wire,
                &mut raw_chunk,
                &mut encoded_chunk,
                &rgba.replace("{id}", "2"),
                0x22,
                rgba_raw_len(),
            );
            push_direct_image(
                &mut wire,
                &mut raw_chunk,
                &mut encoded_chunk,
                &format!("a=f,i=1,f=32,s={FRAME_SIDE},v={FRAME_SIDE}"),
                0x33,
                rgba_raw_len(),
            );
        },
        Case::ComposeFrames => {
            push_direct_image(
                &mut wire,
                &mut raw_chunk,
                &mut encoded_chunk,
                &rgba.replace("{id}", "1"),
                0x11,
                rgba_raw_len(),
            );
            push_direct_image(
                &mut wire,
                &mut raw_chunk,
                &mut encoded_chunk,
                &format!("a=f,i=1,f=32,s={FRAME_SIDE},v={FRAME_SIDE}"),
                0x33,
                rgba_raw_len(),
            );
            push_apc(&mut wire, "a=c,i=1,r=2,c=1,C=1", b"");
        },
        Case::EditFrame => {
            push_direct_image(
                &mut wire,
                &mut raw_chunk,
                &mut encoded_chunk,
                &rgba.replace("{id}", "1"),
                0x11,
                rgba_raw_len(),
            );
            push_direct_image(
                &mut wire,
                &mut raw_chunk,
                &mut encoded_chunk,
                &format!("a=f,i=1,f=32,s={FRAME_SIDE},v={FRAME_SIDE},r=1"),
                0x33,
                rgba_raw_len(),
            );
        },
        Case::DirectRgb => {
            push_direct_image(
                &mut wire,
                &mut raw_chunk,
                &mut encoded_chunk,
                &format!("a=T,i=1,f=24,s={FRAME_SIDE},v={FRAME_SIDE},c=1,r=1,C=1"),
                0x44,
                rgb_raw_len(),
            );
        },
        Case::DirectRgba => {
            push_direct_image(
                &mut wire,
                &mut raw_chunk,
                &mut encoded_chunk,
                &rgba.replace("{id}", "1"),
                0x55,
                rgba_raw_len(),
            );
        },
    }
    wire.shrink_to_fit();
    wire
}

fn drive(term: &mut Term<RecordingListener>, parser: &mut Processor<StdSyncHandler>, wire: &[u8]) {
    let mut remaining = wire;
    while !remaining.is_empty() || parser.has_pending_input() {
        let consumed = parser.advance_until_terminated(term, remaining);
        term.process_graphics_barrier_for_fuzzing();
        remaining = &remaining[consumed..];
    }
}

fn snapshot_pixel_bytes(snapshot: &GraphicsRenderSnapshot) -> usize {
    snapshot.classic.iter().map(|graphic| graphic.pixels.bytes().len()).sum::<usize>()
        + snapshot
            .placeholders
            .iter()
            .map(|placeholder| placeholder.prototype.pixels.bytes().len())
            .sum::<usize>()
}

fn pixels_match(bytes: &[u8], kind: PixelKind) -> bool {
    match kind {
        PixelKind::RgbaPixel(pixel) => {
            bytes.len() == FRAME_BYTES && bytes.chunks_exact(4).all(|value| value == pixel)
        },
        PixelKind::Rgba { fill } => {
            bytes.len() == FRAME_BYTES && bytes.iter().all(|byte| *byte == fill)
        },
        PixelKind::Rgb { fill } => {
            bytes.len() == FRAME_BYTES
                && bytes.chunks_exact(4).all(|pixel| pixel == [fill, fill, fill, 255])
        },
    }
}

fn classify_responses(responses: &[String]) -> (usize, Vec<String>) {
    let mut ok = 0;
    let mut errors = Vec::new();
    for response in responses {
        if response.contains(";OK") {
            ok += 1;
        } else {
            errors.push(response.clone());
        }
    }
    (ok, errors)
}

fn usage() -> i32 {
    eprintln!(
        "usage: kitty_memory_measurement \
         <animation_frame|compose_frames|edit_frame|direct_rgb|direct_rgba>"
    );
    2
}

fn run(case: Case) -> i32 {
    let wire = build_wire(case);
    let listener = RecordingListener::new();
    let recorded = listener.clone();
    let config = Config {
        graphics: GraphicsConfig { storage_limit: case.quota_bytes(), local_transmission: false },
        scrolling_history: 0,
        ..Default::default()
    };
    let mut term = Term::new(config, &TermSize::new(80, 24), listener);
    let mut parser = Processor::<StdSyncHandler>::new();

    let baseline = LIVE.load(ORDERING);
    PEAK.store(baseline, ORDERING);
    drive(&mut term, &mut parser, &wire);
    let peak = PEAK.load(ORDERING).saturating_sub(baseline);
    let live = LIVE.load(ORDERING).saturating_sub(baseline);

    let snapshot = term.graphics_render_snapshot();
    let visible_pixel_bytes = snapshot_pixel_bytes(&snapshot);
    black_box((&wire, &snapshot));

    let (ok, errors) = classify_responses(&recorded.responses());
    let image_count = snapshot.classic.len();
    let pixels_ok = snapshot.classic.len() == 1
        && snapshot
            .classic
            .iter()
            .all(|graphic| pixels_match(graphic.pixels.bytes(), case.expected_pixels()));

    println!("case={}", case.name());
    println!("quota_bytes={}", case.quota_bytes());
    println!("wire_bytes={}", wire.len());
    println!("peak_additional_live_allocation_bytes={peak}");
    println!("live_additional_bytes={live}");
    println!("visible_pixel_bytes={visible_pixel_bytes}");
    println!("expected_retained_pixel_bytes={}", case.quota_bytes());
    println!("snapshot_images={image_count}");
    println!("ok_responses={ok}");
    println!("error_responses={}", errors.len());
    println!("pixels_valid={}", u8::from(pixels_ok));

    if ok != case.expected_ok() || !errors.is_empty() || !pixels_ok {
        for error in &errors {
            eprintln!("graphics_error={error:?}");
        }
        return 1;
    }
    0
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(name) = args.next() else {
        std::process::exit(usage());
    };
    if args.next().is_some() {
        std::process::exit(usage());
    }
    let Some(case) = Case::parse(&name) else {
        std::process::exit(usage());
    };
    std::process::exit(run(case));
}
