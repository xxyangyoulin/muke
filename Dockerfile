FROM php:8.2-cli
RUN apt-get update \
    && apt-get install -y --no-install-recommends imagemagick libonig-dev libzip-dev \
    && docker-php-ext-install mbstring zip \
    && if ! command -v magick >/dev/null; then ln -s "$(command -v convert)" /usr/local/bin/magick; fi \
    && php -r 'foreach (["pdo_sqlite", "sqlite3", "mbstring", "curl", "fileinfo", "zip"] as $ext) { if (!extension_loaded($ext)) { fwrite(STDERR, "Missing extension: $ext\n"); exit(1); } }' \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . .
RUN mkdir -p var
EXPOSE 8765
CMD ["php", "-S", "0.0.0.0:8765", "-d", "upload_max_filesize=16M", "-d", "post_max_size=68M", "-t", "public", "public/index.php"]
