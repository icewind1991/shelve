FROM rust AS build

RUN rustup target add x86_64-unknown-linux-musl

COPY Cargo.toml Cargo.lock ./

# Build with a dummy main to pre-build dependencies
RUN mkdir src && \
 echo "fn main(){}" > src/main.rs && \
 cargo build --release --target x86_64-unknown-linux-musl && \
 rm -r src

COPY src/ ./src/
COPY templates/ ./templates/

RUN touch src/main.rs && cargo build --release --target x86_64-unknown-linux-musl

FROM scratch

COPY --from=build /target/x86_64-unknown-linux-musl/release/shelve /
EXPOSE 80
ENV ROCKET_PORT=80
ENV ROCKET_ADDRESS=0.0.0.0

CMD ["/shelve"]