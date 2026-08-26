use super::*;

fn feed<T: EventListener>(term: &mut Term<T>, input: &[u8]) {
    let mut parser = ansi::Processor::<ansi::StdSyncHandler>::new();
    let mut offset = 0;
    while offset < input.len() || parser.has_pending_input() {
        offset += parser.advance_until_terminated(term, &input[offset..]);
        process_deferred(term);
    }
}

#[test]
fn large_rows_and_relative_offsets_do_not_overflow_deletion() {
    for rows in [i32::MAX as u32, u32::MAX] {
        let mut term = Term::new(Config::default(), &TermSize::new(80, 24), VoidListener);
        let input = format!(
            "\x1b[2;1H\x1b_Ga=T,i=1,s=1,v=1,c=1,r={rows},C=1;/////w==\x1b\\\x1b_Ga=d\x1b\\"
        );
        feed(&mut term, input.as_bytes());
        assert!(term.graphics.placements().next().is_none());
    }
    let mut term = Term::new(Config::default(), &TermSize::new(80, 24), VoidListener);
    feed(&mut term, b"\x1b_Ga=T,i=1,p=1,s=1,v=1,c=1,r=1,C=1;/////w==\x1b\\\x1b_Ga=T,i=2,p=1,P=1,Q=1,V=2147483647,s=1,v=1,c=1,r=1;/////w==\x1b\\\x1b_Ga=d,d=y,y=2147483648\x1b\\");
    assert_eq!(term.graphics.placements().count(), 1);
}

#[test]
fn virtual_parent_origin_uses_the_resolved_prototype() {
    for underline in ["", "\x1b[58;5;7m"] {
        let mut term = Term::new(Config::default(), &TermSize::new(80, 24), VoidListener);
        feed(&mut term, b"\x1b_Ga=T,i=1,p=7,U=1,s=1,v=1,c=1,r=1;/////w==\x1b\\\x1b_Ga=T,i=2,p=1,P=1,Q=7,s=1,v=1,c=1,r=1;/////w==\x1b\\");
        feed(
            &mut term,
            format!("\x1b[5;6H\x1b[38;5;1m{underline}\u{10eeee}\u{305}\u{305}").as_bytes(),
        );
        feed(&mut term, "\x1b[7;3H\u{10eeee}\u{305}\u{305}".as_bytes());
        let snapshot = term.graphics_render_snapshot();
        assert_eq!(snapshot.placeholders.len(), 2);
        assert_eq!(snapshot.classic.len(), 1);
        // The origin combines minimum row and column from different placeholder instances.
        assert_eq!((snapshot.classic[0].line, snapshot.classic[0].column), (Line(4), 2));
        feed(&mut term, b"\x1b_Ga=d,d=p,x=3,y=5\x1b\\");
        assert!(term.graphics_render_snapshot().classic.is_empty());
        assert_eq!(term.graphics_render_snapshot().placeholders.len(), 2);
    }
}

#[test]
fn virtual_rooted_deletion_ignores_the_creation_cursor() {
    let mut term = Term::new(Config::default(), &TermSize::new(80, 24), VoidListener);
    feed(&mut term, b"\x1b_Ga=T,i=1,p=7,U=1,s=1,v=1,c=1,r=1;/////w==\x1b\\\x1b_Ga=T,i=2,p=1,P=1,Q=7,s=1,v=1,c=1,r=1;/////w==\x1b\\");
    feed(&mut term, "\x1b[5;6H\x1b[38;5;1;58;5;7m\u{10eeee}\u{305}\u{305}".as_bytes());
    feed(&mut term, b"\x1b_Ga=d,d=p,x=1,y=1\x1b\\");
    assert_eq!(term.graphics_render_snapshot().classic.len(), 1);
    feed(&mut term, b"\x1b[2J");
    assert!(term.graphics_render_snapshot().classic.is_empty());
}

#[test]
fn native_sizing_survives_font_changes_and_updates_its_footprint() {
    for axes in ["", ",c=0", ",r=0", ",c=0,r=0"] {
        let mut term = Term::new(Config::default(), &TermSize::new(80, 24), VoidListener);
        term.set_cell_dimensions(1, 1);
        let input =
            format!("\x1b_Ga=T,i=1,s=3,v=2,C=1{axes};////////////////////////////////\x1b\\");
        feed(&mut term, input.as_bytes());
        let snapshot = term.graphics_render_snapshot();
        assert_eq!(snapshot.classic.len(), 1);
        assert_eq!((snapshot.classic[0].columns, snapshot.classic[0].rows), (None, None));
        term.set_cell_dimensions(10, 20);
        let snapshot = term.graphics_render_snapshot();
        assert_eq!((snapshot.classic[0].columns, snapshot.classic[0].rows), (None, None));
        feed(&mut term, b"\x1b_Ga=d,d=p,x=2,y=1\x1b\\");
        assert_eq!(term.graphics_render_snapshot().classic.len(), 1);
        feed(&mut term, b"\x1b_Ga=d,d=p,x=1,y=1\x1b\\");
        assert!(term.graphics_render_snapshot().classic.is_empty());
    }
}

#[test]
fn self_parent_returns_cycle_and_preserves_the_placement() {
    let listener = RecordingListener::default();
    let mut term = Term::new(Config::default(), &TermSize::new(80, 24), listener.clone());
    feed(&mut term, b"\x1b_Ga=T,i=1,p=1,s=1,v=1,C=1;/////w==\x1b\\\x1b_Ga=p,i=1,p=1,P=1,Q=1\x1b\\");
    assert_eq!(term.graphics_render_snapshot().classic.len(), 1);
    assert_eq!(listener.0.lock().unwrap().last().unwrap(), "\x1b_Gi=1,p=1;ECYCLE\x1b\\");
}
