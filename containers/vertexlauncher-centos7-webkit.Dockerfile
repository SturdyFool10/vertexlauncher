FROM ubuntu:22.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update  && apt-get install -y --no-install-recommends   ca-certificates   curl   build-essential   pkg-config   patchelf   file   desktop-file-utils   binutils   libglib2.0-dev   libgtk-3-dev   libgdk-pixbuf-2.0-dev   libpango1.0-dev   libatk1.0-dev   libcairo2-dev   libdbus-1-dev   libsoup2.4-dev   libwebkit2gtk-4.1-dev   libjavascriptcoregtk-4.1-dev   libudev-dev  && rm -rf /var/lib/apt/lists/*
