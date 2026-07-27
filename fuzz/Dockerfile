FROM rust:1-bookworm

RUN cargo install cargo-afl

WORKDIR /app

COPY . .

RUN cd fuzz && cargo afl build --release

CMD ["bash", "fuzz/deploy.sh"]
