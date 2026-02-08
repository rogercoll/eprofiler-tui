#[allow(warnings)]
pub mod opentelemetry {
    pub mod proto {
        pub mod common {
            pub mod v1 {
                tonic::include_proto!("opentelemetry.proto.common.v1");
            }
        }
        pub mod resource {
            pub mod v1 {
                tonic::include_proto!("opentelemetry.proto.resource.v1");
            }
        }
        pub mod profiles {
            pub mod v1development {
                tonic::include_proto!("opentelemetry.proto.profiles.v1development");
            }
        }
        pub mod collector {
            pub mod profiles {
                pub mod v1development {
                    tonic::include_proto!(
                        "opentelemetry.proto.collector.profiles.v1development"
                    );
                }
            }
        }
    }
}
