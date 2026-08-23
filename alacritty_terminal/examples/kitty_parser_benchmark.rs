use std::hint::black_box;
use std::time::Instant;

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, GraphicsConfig, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

struct Size;

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        24
    }

    fn screen_lines(&self) -> usize {
        24
    }

    fn columns(&self) -> usize {
        80
    }
}

fn main() {
    let enabled = std::env::args().nth(1).as_deref() == Some("enabled");
    let iterations =
        std::env::args().nth(2).and_then(|value| value.parse::<usize>().ok()).unwrap_or(200_000);
    let config =
        Config { graphics: GraphicsConfig { enabled, ..Default::default() }, ..Default::default() };
    let mut term = Term::new(config, &Size, VoidListener);
    let mut parser = Processor::<StdSyncHandler>::new();
    let input =
        b"prompt $ printf '\\e[31mordinary terminal text\\e[0m\\n'\r\nordinary terminal text\r\n";

    let start = Instant::now();
    for _ in 0..iterations {
        parser.advance(&mut term, black_box(input));
    }
    println!("{}", start.elapsed().as_nanos());
}
