fn main() {
    let vendor = std::path::Path::new("vendor");

    let sql_dir = vendor.join("tree-sitter-sql");
    println!("cargo:rerun-if-changed={}", sql_dir.display());
    cc::Build::new()
        .include(&sql_dir)
        .file(sql_dir.join("parser.c"))
        .warnings(false)
        .compile("tree-sitter-sql-parser");

    cc::Build::new()
        .include(&sql_dir)
        .file(sql_dir.join("scanner.cc"))
        .cpp(true)
        .warnings(false)
        .compile("tree-sitter-sql-scanner");

    let proto_dir = vendor.join("tree-sitter-proto");
    println!("cargo:rerun-if-changed={}", proto_dir.display());
    cc::Build::new()
        .include(&proto_dir)
        .file(proto_dir.join("parser.c"))
        .warnings(false)
        .compile("tree-sitter-proto");

    for name in ["dockerfile", "astro", "scss", "vue"] {
        let dir = vendor.join(format!("tree-sitter-{name}"));
        let parser = dir.join("parser.c");
        let scanner = dir.join("scanner.c");
        println!("cargo:rerun-if-changed={}", dir.display());
        cc::Build::new()
            .include(&dir)
            .file(&parser)
            .warnings(false)
            .compile(&format!("tree-sitter-{name}-parser"));

        cc::Build::new()
            .include(&dir)
            .file(&scanner)
            .warnings(false)
            .compile(&format!("tree-sitter-{name}-scanner"));
    }
}
