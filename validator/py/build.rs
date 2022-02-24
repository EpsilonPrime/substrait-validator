use std::collections::HashSet;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use walkdir::WalkDir;

fn main() {
    // Directory that the proto files are stored in.
    let input_path = "../../proto";

    // Output directory for protoc. This is a temporary directory: it will be
    // completely deleted and then reconstructed. Afterward, the build script
    // will patch the files in here and then move them to python_out.
    let intermediate_path = "protoc_out";

    // Where the final Python files will be moved to.
    let output_path = "substrait_validator";

    // The Python module prefix to patch into use statements of the files
    // generated by protobuf.
    let python_prefix = "substrait_validator.";

    // Canonicalize all paths to prevent ambiguity.
    let input_path = dunce::canonicalize(PathBuf::from(input_path)).unwrap();
    let workdir = std::env::current_dir().unwrap();
    let intermediate_path = workdir.join(intermediate_path);
    let output_path = workdir.join(output_path);

    // Gather all .proto files.
    let proto_files: Vec<_> = WalkDir::new(&input_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension() == Some(OsStr::new("proto")) && e.metadata().unwrap().is_file()
        })
        .map(|e| dunce::canonicalize(e.into_path()).unwrap())
        .collect();

    // Clean and recreate output directory.
    fs::remove_dir_all(&intermediate_path).ok();
    fs::create_dir_all(&intermediate_path).expect("failed to create protoc output directory");

    // Run protoc.
    let mut proto_path_arg = OsString::new();
    proto_path_arg.push("--proto_path=");
    proto_path_arg.push(&input_path);
    let mut python_out_arg = OsString::new();
    python_out_arg.push("--python_out=");
    python_out_arg.push(&intermediate_path);
    let protoc = prost_build::protoc();
    let mut cmd = Command::new(protoc);
    cmd.arg(proto_path_arg);
    cmd.arg(python_out_arg);
    cmd.args(proto_files.iter());
    let output = cmd.output().expect("failed to run protoc");
    if !output.status.success() {
        eprintln!("cmd: {:?}", cmd.get_program());
        for arg in cmd.get_args() {
            eprintln!("arg: {:?}", arg);
        }
        panic!("{:?}", output);
    }

    // Gather all Python files generated by protoc.
    let intermediate_files: Vec<_> = WalkDir::new(&intermediate_path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().extension() == Some(OsStr::new("py")) && e.metadata().unwrap().is_file()
        })
        .map(|e| dunce::canonicalize(e.into_path()).unwrap())
        .collect();

    // Patch the files.
    let mut output_dirs = HashSet::new();
    for intermediate_file in intermediate_files {
        // Determine the output filename.
        let output_file = output_path.join(
            intermediate_file
                .strip_prefix(&intermediate_path)
                .expect("intermediate file is not based in the expected directory"),
        );

        // Determine the output directory.
        let mut path = output_file.to_path_buf();
        path.pop();

        // Ensure that the directory exists, and create an __init__.py for it
        // if we haven't already.
        let mut path = output_file.to_path_buf();
        path.pop();
        if output_dirs.insert(path.clone()) {
            fs::create_dir_all(&path).expect("failed to create output directory");
            path.push("__init__.py");
            fs::File::create(path).expect("failed to create __init__.py");
        }

        // Copy and patch the file.
        let intermediate =
            fs::File::open(&intermediate_file).expect("failed to open intermediate file");
        let mut output = fs::File::create(&output_file).expect("failed to create output file");
        for line in BufReader::new(intermediate).lines() {
            let line = line.expect("failed to read from intermediate file");
            let line = if line.starts_with("from ") && !line.starts_with("from google") {
                format!("from {}{}", python_prefix, &line[5..])
            } else {
                line
            };
            writeln!(output, "{}", line).unwrap();
        }
    }
}
