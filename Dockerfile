# Build, test, and run the lifecycle gate as an ordinary container user.
ARG RUST_IMAGE=rust:1.98.1-slim-bookworm
FROM ${RUST_IMAGE} AS foundation
WORKDIR /work
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
COPY examples ./examples
COPY tests ./tests
COPY LICENSE NOTICE README.md ./
COPY docs ./docs
RUN cargo test --locked
RUN cargo build --release --locked --bin spanforge-verify --bin cliverifyr --bin spanforge-verify-fixture --bin linux-gate
RUN cargo metadata --locked --format-version 1 --filter-platform x86_64-unknown-linux-gnu > /work/dependency-inventory.json \
    && rustc --version --verbose > /work/compiler.txt \
    && sha256sum target/release/spanforge-verify target/release/cliverifyr > /work/SHA256SUMS \
    && mkdir /work/third-party-licenses \
    && for package_dir in "$CARGO_HOME"/registry/src/*/*; do \
         [ -d "$package_dir" ] || continue; \
         package_name=$(basename "$package_dir"); \
         mkdir "/work/third-party-licenses/$package_name"; \
         for notice in "$package_dir"/LICENSE* "$package_dir"/COPYING* "$package_dir"/NOTICE*; do \
           if [ -f "$notice" ]; then cp "$notice" "/work/third-party-licenses/$package_name/"; fi; \
         done; \
       done
RUN mkdir /evidence && chown 65532:65532 /evidence
USER 65532:65532
RUN target/release/linux-gate --fixture target/release/spanforge-verify-fixture --repeat 100 --evidence /evidence/linux-gate.json

FROM debian:bookworm-slim AS cli
COPY --from=foundation /work/target/release/spanforge-verify /usr/local/bin/spanforge-verify
COPY --from=foundation /work/target/release/cliverifyr /usr/local/bin/cliverifyr
COPY --from=foundation /work/LICENSE /work/NOTICE /usr/share/spanforge-verify/
COPY --from=foundation /work/dependency-inventory.json /work/compiler.txt /work/SHA256SUMS /usr/share/spanforge-verify/
COPY --from=foundation /work/third-party-licenses /usr/share/spanforge-verify/third-party-licenses
COPY --from=foundation /work/docs /usr/share/spanforge-verify/docs
COPY --from=foundation /work/examples /usr/share/spanforge-verify/examples
USER 65532:65532
WORKDIR /tmp
ENTRYPOINT ["spanforge-verify"]
CMD ["--help"]
