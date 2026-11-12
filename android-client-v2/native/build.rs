//! prost codegen for the frozen control-plane schema.
//!
//! Single source of truth: `protos/wire-protocol-v2/control.proto` (the same
//! file the Kotlin side compiles with protobuf-kotlin-lite — two ends, one
//! schema, no hand-written codec). The crate sits at
//! `android-client-v2/native/`, so the schema is two directories up.

fn main() {
    let proto = "../../protos/wire-protocol-v2/control.proto";
    prost_build::Config::new()
        .compile_protos(&[proto], &["../../protos/wire-protocol-v2"])
        .expect("compile protos/wire-protocol-v2/control.proto");
}
