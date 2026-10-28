fn main() {
    glib_build_tools::compile_resources(
        &["data/resources"],
        "data/resources/runetty.gresource.xml",
        "runetty.gresource",
    );
}
