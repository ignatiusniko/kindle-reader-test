//! Write the synthetic fixture set of spec §26.2 to a directory.
//!
//! Usage: cargo run -p test-fixtures --bin make-fixtures -- [OUT_DIR] [--stress]
//! Never run the oversized generators on the Kindle itself.

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let stress = args.iter().any(|a| a == "--stress");
    let out = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("tests/fixtures"));
    std::fs::create_dir_all(&out)?;
    let write = |name: &str, bytes: Vec<u8>| -> std::io::Result<()> {
        let p = out.join(name);
        std::fs::write(&p, &bytes)?;
        println!("{:>10} bytes  {}", bytes.len(), p.display());
        Ok(())
    };
    use test_fixtures as f;
    write("sample_12_pages.cbz", f::jpeg_book(12, 1200, 1800))?;
    write("normal_jpeg.cbz", f::jpeg_book(20, 1000, 1500))?;
    write("normal_png.cbz", f::png_book(10, 1000, 1500))?;
    write("rtl_order.cbz", f::rtl_order_book())?;
    write("corrupt_page.cbz", f::corrupt_page_book(5, 3))?;
    write("bad_zip.cbz", f::bad_zip())?;
    write("suspicious_ratio.cbz", f::suspicious_ratio_book())?;
    write("long_index.cbz", f::long_index_book(1200))?;
    if stress {
        // Large dimensions, small files. Host only.
        write("oversized_jpeg.cbz", f::oversized_jpeg_book(8000, 12000))?;
        write("oversized_png.cbz", f::oversized_png_book(10000, 20000))?;
    }
    Ok(())
}
