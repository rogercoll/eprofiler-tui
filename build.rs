fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                "opentelemetry-proto/opentelemetry/proto/collector/profiles/v1development/profiles_service.proto",
            ],
            &["opentelemetry-proto"],
        )?;
    Ok(())
}
