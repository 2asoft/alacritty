#![no_main]

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::graphics::process_request;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::{Config, GraphicsConfig, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use libfuzzer_sys::fuzz_target;

struct Size {
    columns: usize,
    lines: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

fuzz_target!(|data: &[u8]| {
    let size = Size { columns: 16, lines: 8 };
    let config = Config {
        graphics: GraphicsConfig {
            storage_limit: 64 * 1024,
            local_transmission: false,
        },
        ..Default::default()
    };
    let mut term = Term::new(config, &size, VoidListener);
    let mut parser = Processor::<StdSyncHandler>::new();
    let mut remaining = data;
    while !remaining.is_empty() || parser.has_pending_input() {
        let consumed = parser.advance_until_terminated(&mut term, remaining);
        if let Some(request) = term.take_graphics_request() {
            term.begin_graphics_processing(&request);
            let options = term.graphics_processing_options_for(&request);
            term.commit_graphics_command(process_request(request, options.0, options.1));
        }
        remaining = &remaining[consumed..];
        if consumed == 0 && !parser.has_pending_input() {
            break;
        }
    }

    parser.stop_sync(&mut term);
    loop {
        if let Some(request) = term.take_graphics_request() {
            term.begin_graphics_processing(&request);
            let options = term.graphics_processing_options_for(&request);
            term.commit_graphics_command(process_request(request, options.0, options.1));
        }
        if !parser.has_pending_input() {
            break;
        }
        let _ = parser.advance_until_terminated(&mut term, b"");
    }

    if data.first().is_some_and(|byte| byte & 2 != 0) {
        parser.advance(&mut term, b"\x18\x1b[2J\x1b[?1049h\x1b[?1049l\x1bc");
    }
    if data.first().is_some_and(|byte| byte & 4 != 0) {
        for _ in 0..usize::from(data.get(3).copied().unwrap_or(0) % 16) {
            parser.advance(&mut term, b"\n");
        }
    }
    let _ = term.advance_graphics_animations(std::time::Instant::now());

    if data.first().is_some_and(|byte| byte & 1 != 0) {
        let columns = usize::from(data.get(1).copied().unwrap_or(0) % 31 + 2);
        let lines = usize::from(data.get(2).copied().unwrap_or(0) % 15 + 2);
        term.resize(Size { columns, lines });
    }
    let _ = term.grid()[Line(0)][Column(0)].c;
});
