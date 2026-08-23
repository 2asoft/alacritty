# Terminology

- **image**: Canonical CPU-side pixel and animation-frame data. It is independent of display location.
- **placement**: A terminal object that displays an image with source, destination, anchor, and z-order properties.
- **classic placement**: A placement owned by graphics state rather than a terminal cell.
- **virtual placement**: Placement metadata used by Unicode placeholder cells. It does not render directly.
- **placeholder cell**: An ordinary cell containing U+10EEEE and protocol attributes which resolves through a virtual placement.
- **tracked anchor**: A terminal-content position transformed with scrolling, history pruning, and reflow.
- **screen buffer**: The primary or alternate grid together with its matching graphics state.
- **canonical pixels**: Straight-alpha RGBA8 sRGB bytes retained by terminal state.
- **renderer cache**: Reconstructible GPU textures keyed by image identity, frame, and content generation.
- **deferred graphics request**: An ordered protocol operation processed outside the terminal mutex before later PTY bytes advance.
