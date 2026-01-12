# inspiration: https://dev.to/rogertorres/first-steps-with-docker-rust-30oi

FROM rust:1.92-bookworm AS build

# create an empty shell project
RUN USER=root cargo new --bin mc-frontend
WORKDIR /mc-frontend

# copy manifests
COPY ./Cargo.lock ./Cargo.lock
COPY ./Cargo.toml ./Cargo.toml
RUN cargo build --release
RUN rm -r ./src

# copy real source
COPY ./src ./src
COPY ./templates ./templates

# build for release
RUN rm ./target/release/mc-frontend*
RUN find ./src -name '*rs' -exec touch '{}' \;
RUN cargo build --release

# executing image
FROM debian:bookworm-slim

RUN apt update && apt install -y ca-certificates
COPY --from=build /mc-frontend/target/release/mc-frontend mc-frontend
COPY ./assets ./assets

ENTRYPOINT ["./mc-frontend"]
