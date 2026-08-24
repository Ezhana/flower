use std::{env, fs, path::Path};

use anyhow::{Context, Result, bail};
use wasmparser::{Encoding, Parser, Payload, Validator, WasmFeatures};
use wit_component::ComponentEncoder;

fn main() -> Result<()> {
    let arguments = env::args().collect::<Vec<_>>();
    match arguments.as_slice() {
        [_, command, input, output] if command == "new" => component_new(input, output),
        [_, command, input] if command == "validate" => validate(input),
        _ => bail!("usage: flower-component-tools <new INPUT OUTPUT|validate INPUT>"),
    }
}

fn component_new(input: &str, output: &str) -> Result<()> {
    let module = fs::read(input).with_context(|| format!("read core module {input}"))?;
    let component = ComponentEncoder::default()
        .validate(true)
        .module(&module)
        .context("embed component type metadata")?
        .encode()
        .context("encode component")?;
    if let Some(parent) = Path::new(output).parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create component output directory {}", parent.display()))?;
    }
    fs::write(output, component).with_context(|| format!("write component {output}"))
}

fn validate(input: &str) -> Result<()> {
    let bytes = fs::read(input).with_context(|| format!("read component {input}"))?;
    Validator::new_with_features(WasmFeatures::all())
        .validate_all(&bytes)
        .context("validate WebAssembly component")?;
    let mut imports = Vec::new();
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload.context("parse component")? {
            Payload::Version {
                encoding: Encoding::Component,
                ..
            } => {}
            Payload::ComponentImportSection(section) => {
                for import in section {
                    imports.push(import.context("parse component import")?.name.0.to_owned());
                }
            }
            _ => {}
        }
    }
    if imports.iter().any(|name| name.starts_with("wasi:")) {
        bail!("component imports WASI: {}", imports.join(", "));
    }
    println!("valid component; WASI imports: none");
    Ok(())
}
