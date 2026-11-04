# syntax=docker/dockerfile:1
# Shoebox Cloud, self-hosted (docs/notes/cloud-deploy.md). Built from the repository root:
#   docker build -f deploy/selfhost/Dockerfile -t shoebox-cloud .
# or with deploy/selfhost/compose.yaml. The image holds the service and the viewer page; the data is in /data.

FROM rust:1-bookworm AS build
# The viewer page is WebAssembly: the wasm32 target, wasm-bindgen at the version Cargo.lock pins, and a clang that
# targets wasm32 (ring, in the Live-link client).
RUN apt-get update && apt-get install -y --no-install-recommends clang llvm && rm -rf /var/lib/apt/lists/* \
 && rustup target add wasm32-unknown-unknown
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install wasm-bindgen-cli --version 0.2.129 --locked --root /opt/tools
WORKDIR /src
COPY . .
ENV WASM_BINDGEN=/opt/tools/bin/wasm-bindgen \
    CC_wasm32_unknown_unknown=clang \
    AR_wasm32_unknown_unknown=llvm-ar
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    scripts/build-viewer.sh \
 && cargo build --release --locked -p shoebox-cloud --bin shoebox-cloud \
 && mkdir -p /out \
 && cp target/release/shoebox-cloud /out/ \
 && cp -r apps/web-viewer/dist /out/viewer \
 && mkdir /out/data

# No shell and no root: the service needs only its binary, the page and the data volume.
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /out/shoebox-cloud /usr/local/bin/shoebox-cloud
COPY --from=build /out/viewer /srv/viewer
# A new volume takes this directory's owner, so the service (uid 65532) can write to it.
COPY --from=build --chown=65532:65532 /out/data /data
ENV SHOEBOX_CLOUD_DATA=/data \
    SHOEBOX_CLOUD_LISTEN=0.0.0.0:8080 \
    SHOEBOX_CLOUD_VIEWER=/srv/viewer
VOLUME /data
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/shoebox-cloud"]
