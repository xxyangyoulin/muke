FROM rust:1.91-slim-bookworm AS build
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends build-essential perl pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY plugins ./plugins
RUN cargo build --release --locked
COPY frontend ./frontend

FROM debian:bookworm-slim
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends imagemagick ca-certificates && rm -rf /var/lib/apt/lists/* \
    && ln -s /usr/bin/convert /usr/local/bin/magick \
    && useradd --system --home /opt/muke muke
WORKDIR /opt/muke
COPY --from=build /src/target/release/muke /usr/local/bin/muke
COPY scripts/muke.toml.example /opt/muke/muke.toml
COPY --from=build /src/frontend /opt/muke/frontend
COPY --from=build /src/plugins /opt/muke/plugins
RUN mkdir -p /opt/muke/data /opt/muke/dist && chown -R muke:muke /opt/muke
USER muke
EXPOSE 8785
ENV MKBLOG_DATA_DIR=/opt/muke/data
ENTRYPOINT ["muke"]
CMD ["--config", "/opt/muke/muke.toml", "serve", "--listen", "0.0.0.0:8785"]
