fn main() {
    let sql_dir = std::path::Path::new("../tree-sitter-sql/src");
    if sql_dir.exists() {
        println!("cargo:rerun-if-changed=../tree-sitter-sql/src/parser.c");
        println!("cargo:rerun-if-changed=../tree-sitter-sql/src/scanner.cc");
        cc::Build::new()
            .include(sql_dir)
            .file(sql_dir.join("parser.c"))
            .warnings(false)
            .compile("tree-sitter-sql-parser");

        cc::Build::new()
            .include(sql_dir)
            .file(sql_dir.join("scanner.cc"))
            .cpp(true)
            .warnings(false)
            .compile("tree-sitter-sql-scanner");
    }

    let proto_dir = std::path::Path::new("../tree-sitter-proto/src");
    if proto_dir.exists() {
        println!("cargo:rerun-if-changed=../tree-sitter-proto/src/parser.c");
        cc::Build::new()
            .include(proto_dir)
            .file(proto_dir.join("parser.c"))
            .warnings(false)
            .compile("tree-sitter-proto");
    }

    // These grammars are not tracked in git; scripts/bootstrap-vendored-grammars.sh
    // downloads them. Fail at build-script time instead of at link time.
    for name in ["dockerfile", "astro", "scss", "vue"] {
        let dir = std::path::PathBuf::from(format!("../tree-sitter-{name}/src"));
        let parser = dir.join("parser.c");
        let scanner = dir.join("scanner.c");
        assert!(
            parser.exists() && scanner.exists(),
            "tree-sitter-{name} grammar not found. Run scripts/bootstrap-vendored-grammars.sh to download it."
        );
        println!("cargo:rerun-if-changed={}", parser.display());
        cc::Build::new()
            .include(&dir)
            .file(&parser)
            .warnings(false)
            .compile(&format!("tree-sitter-{name}-parser"));

        println!("cargo:rerun-if-changed={}", scanner.display());
        cc::Build::new()
            .include(&dir)
            .file(&scanner)
            .warnings(false)
            .compile(&format!("tree-sitter-{name}-scanner"));
    }
}
