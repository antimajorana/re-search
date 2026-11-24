# Installer notice sources

Run the generator after the release build, using the same Cargo and Rust toolchain. The toolchain's `rust-docs` component supplies the standard library copyright report.

```text
python scripts/generate-third-party-notices.py --target x86_64-pc-windows-msvc --output-dir path/to/staging/licenses
```

- The WebView2 Loader license and notice come from the official Microsoft.Web.WebView2 1.0.3800.47 NuGet archive. The x64 import library, DLL, and static library in locked `webview2-com-sys` 0.39.1 matched the NuGet archive byte for byte. [Microsoft.Web.WebView2](https://www.nuget.org/packages/Microsoft.Web.WebView2/1.0.3800.47)
- The locked `webview2-com` crates omit their MIT text in the Cargo archives. The upstream license covers the Rust bindings and procedural macros. [webview2-rs license](https://github.com/wravery/webview2-rs/blob/edc2caf886175ccaebe86078c9cfe1ae2a187328/LICENSE)
- The KaTeX 0.19.1 browser bundle matches the npm release, including all 20 WOFF2 files. The fonts' name tables supply the copyright holders, reserved names, and SIL Open Font License 1.1 designation. [KaTeX npm release](https://www.npmjs.com/package/katex/v/0.19.1) · [SIL Open Font License 1.1](https://openfontlicense.org/open-font-license-official-text/)
- The PDF reader's generated glyph table identifies the Adobe glyph mappings and Apache PDFBox data. The installer includes their upstream notices and the Apache License 2.0. [Adobe Glyph List](https://github.com/adobe-type-tools/agl-aglfn/blob/master/glyphlist.txt) · [PDFBox notice](https://github.com/apache/pdfbox/blob/trunk/NOTICE.txt)
- The NSIS installer uses the zlib compressor and stock NSIS components. Their notices come from the upstream NSIS COPYING and zlib LICENSE. [NSIS COPYING](https://github.com/NSIS-Dev/nsis/blob/v311/COPYING) · [zlib LICENSE](https://github.com/madler/zlib/blob/v1.3.1/LICENSE)
