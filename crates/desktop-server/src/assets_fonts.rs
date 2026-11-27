//! Embedded offline KaTeX WOFF2 font assets split out of `assets.rs`
//! to keep `assets.rs` well under the 600-line ceiling (`RULES.md`).

use crate::assets::StaticAsset;

pub(crate) fn lookup_katex_font(path: &str) -> Option<StaticAsset> {
    match path {
        "/vendor/katex/fonts/KaTeX_AMS-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_AMS-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Caligraphic-Bold.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Caligraphic-Bold.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Caligraphic-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Caligraphic-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Fraktur-Bold.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Fraktur-Bold.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Fraktur-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Fraktur-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Main-Bold.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Main-Bold.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Main-BoldItalic.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Main-BoldItalic.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Main-Italic.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Main-Italic.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Main-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Main-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Math-BoldItalic.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Math-BoldItalic.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Math-Italic.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Math-Italic.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_SansSerif-Bold.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_SansSerif-Bold.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_SansSerif-Italic.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_SansSerif-Italic.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_SansSerif-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_SansSerif-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Script-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Script-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Size1-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Size1-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Size2-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Size2-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Size3-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Size3-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Size4-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Size4-Regular.woff2"),
        }),
        "/vendor/katex/fonts/KaTeX_Typewriter-Regular.woff2" => Some(StaticAsset {
            content_type: "font/woff2",
            bytes: include_bytes!("../vendor/katex/fonts/KaTeX_Typewriter-Regular.woff2"),
        }),
        _ => None,
    }
}
