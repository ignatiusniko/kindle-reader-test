//! G2 build / G4 run: minimal Slint test window (spec §20.3).
//!
//! Transfer this to the Kindle after G1, but **do not launch it** until the
//! G3 display hand-off/restore procedure has been written and rehearsed
//! (docs/safety-and-recovery.md).

slint::slint! {
    export component AppWindow inherits Window {
        in-out property <int> taps: 0;
        in property <string> info;
        callback exit_requested();

        background: white;

        VerticalLayout {
            padding: 48px;
            spacing: 36px;
            alignment: center;

            Text {
                text: "E-Ink Reader: hardware test";
                font-size: 48px;
                color: black;
                horizontal-alignment: center;
            }
            Text {
                text: "Touch count: " + root.taps;
                color: black;
                font-size: 44px;
                horizontal-alignment: center;
            }
            // Grey steps for UI-05 (light/mid/dark grey check).
            HorizontalLayout {
                height: 80px;
                for level in [0, 34, 68, 102, 136, 170, 204, 238]: Rectangle {
                    background: rgb(level, level, level);
                }
            }
            tap := Rectangle {
                height: 160px;
                border-width: 4px;
                border-color: black;
                background: tap-area.pressed ? black : white;
                Text { text: "Tap test"; font-size: 44px; color: tap-area.pressed ? white : black; }
                tap-area := TouchArea { clicked => { root.taps += 1; } }
            }
            exit := Rectangle {
                height: 160px;
                border-width: 4px;
                border-color: black;
                background: exit-area.pressed ? black : white;
                Text { text: "Exit to Kindle"; font-size: 44px; color: exit-area.pressed ? white : black; }
                exit-area := TouchArea { clicked => { root.exit_requested(); } }
            }
            Text {
                text: root.info;
                font-size: 24px;
                color: #444;
                horizontal-alignment: center;
                wrap: word-wrap;
            }
        }
        // Corner markers for UI-03 (touch mapping / orientation).
        for corner in [[0, 0], [1, 0], [0, 1], [1, 1]]: Rectangle {
            x: corner[0] * (root.width - 120px);
            y: corner[1] * (root.height - 120px);
            width: 120px;
            height: 120px;
            border-width: 4px;
            border-color: black;
            background: corner-area.pressed ? black : transparent;
            corner-area := TouchArea { clicked => { root.taps += 1; } }
        }
    }
}

#[cfg(feature = "kindle")]
static FONT: &[u8] = include_bytes!("../../../assets/fonts/ReaderFont.ttf");

#[cfg(not(any(feature = "kindle", feature = "desktop")))]
fn main() {
    eprintln!("eink-reader-m0: built without a display backend (use --features kindle or desktop)");
    std::process::exit(2);
}

#[cfg(any(feature = "kindle", feature = "desktop"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "kindle")]
    let _backend = {
        // This may fail on KT5 until the backend's device support is verified.
        let b = slint_backend_kindle::install(FONT)?;
        b.set_black_and_white(false);
        b
    };
    let app = AppWindow::new()?;
    app.set_info(
        format!(
            "build {} · {}-{} · every tap must add exactly one",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH,
            std::env::consts::OS
        )
        .into(),
    );
    app.on_exit_requested(|| {
        let _ = slint::quit_event_loop();
    });
    let started = std::time::Instant::now();
    app.run()?;
    eprintln!(
        "eink-reader-m0: exit after {:.1}s, taps={}",
        started.elapsed().as_secs_f32(),
        app.get_taps()
    );
    Ok(())
}
