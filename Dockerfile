ARG rust-ver
ARG image
ARG image-ver

FROM rust:$rust-ver AS build
COPY . /build
WORKDIR /build
RUN apt-get update && apt-get upgrade -y && apt-get install libssl-dev pkg-config -y
RUN cargo build --release

FROM $image:$image-ver
RUN apt-get update && apt-get upgrade -y && apt-get install libssl3 -y
COPY --from=build /build/target/release/jet /usr/bin/
