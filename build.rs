extern crate protobuf_codegen_pure;

use std::env;
use std::fs::File;
use std::io::BufReader;
use std::io::prelude::*;

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    protobuf_codegen_pure::run(protobuf_codegen_pure::Args {
        out_dir: &out_dir,
        input: &["proto/message_wire.proto"],
        includes: &["proto"],
        customize: protobuf_codegen_pure::Customize {
            ..Default::default()
        }
    }).expect("protoc");

    // FIXME: Included files, such as the one we make above, can't contain
    // inner attributes (Rust issue #18810). To remedy this, we just manually
    // insert the attributes before the include, and strip them out of the file
    // now.
    let temp_name = format!("{}/message_wire.rs.temp", out_dir);
    let gen_name = format!("{}/message_wire.rs", out_dir);
    {
        let mut temp = File::create(&temp_name).unwrap();
        let generated = File::open(&gen_name).unwrap();
        let br = BufReader::new(generated);
        for line in br.lines() {
            let line = line.unwrap();
            if line.starts_with("#!") || line.starts_with("//!") {
                // throw it away
                continue;
            }
            temp.write_all(&line.as_bytes()).unwrap();
            temp.write_all(b"\n").unwrap();
        }
    }
    std::fs::rename(&temp_name, &gen_name).unwrap();
}
