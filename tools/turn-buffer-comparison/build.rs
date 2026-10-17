use std::{env, fs, path::PathBuf};

fn main() {
    let source = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../src-native/vendor/webrtc-ice-0.14.0/src/turn_stream.rs");
    println!("cargo:rerun-if-changed={}", source.display());
    let original = fs::read_to_string(source).unwrap();
    // Compile the actual production implementation; only expose fixture construction.
    let base = original
        .lines()
        .filter(|s| !s.starts_with("//!"))
        .collect::<Vec<_>>()
        .join("\n");
    let fixture = r#"
pub fn fixture(stream: impl AsyncRead + AsyncWrite + Unpin + Send + Sync + 'static, local: SocketAddr, remote: SocketAddr) -> TurnStream {
    TurnStream::from_stream(Box::new(stream), local, remote)
}
"#;
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    fs::write(out.join("baseline.rs"), format!("{base}\n{fixture}")).unwrap();

    let start = base
        .find("                let mut header = [0; 4];")
        .unwrap();
    let end = base[start..]
        .find("                Ok::<_, io::Error>(size)")
        .unwrap()
        + start
        + "                Ok::<_, io::Error>(size)".len();
    let mut buffered = base.clone();
    buffered.replace_range(start..end, "                reader.receive(buffer).await");
    assert_eq!(
        buffered.matches("reader: Mutex<ReadHalf<Stream>>").count(),
        1
    );
    buffered = buffered
        .replace(
            "reader: Mutex<ReadHalf<Stream>>",
            "reader: Mutex<BufferedReader>",
        )
        .replace(
            "reader: Mutex::new(reader)",
            "reader: Mutex::new(BufferedReader::new(reader))",
        )
        .replace(
            "writer: Mutex<WriteHalf<Stream>>",
            "writer: Mutex<BufferedWriter>",
        )
        .replace(
            "writer: Mutex::new(writer)",
            "writer: Mutex::new(BufferedWriter::new(writer))",
        );
    let helpers = fs::read_to_string("src/buffered.inc").unwrap();
    println!("cargo:rerun-if-changed=src/buffered.inc");
    fs::write(
        out.join("buffered.rs"),
        format!("{buffered}\n{fixture}\n{helpers}"),
    )
    .unwrap();
}
