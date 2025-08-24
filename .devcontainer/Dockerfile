ARG VARIANT=bookworm-slim
FROM debian:${VARIANT}
ENV DEBIAN_FRONTEND=noninteractive

# Arguments
ARG CONTAINER_USER=vscode
ARG CONTAINER_GROUP=vscode

# Install dependencies
RUN apt-get update \
    && apt-get install -y \
        build-essential \
        clang \
        cmake \
        curl \
        file \
        git \
        libayatana-appindicator3-dev \
        librsvg2-dev \
        libssl-dev \
        libwebkit2gtk-4.1-dev \
        libxdo-dev \
        pkg-config \
        wget \
    && apt-get clean -y && rm -rf /var/lib/apt/lists/* /tmp/library-scripts

# Set users
RUN adduser --disabled-password --gecos "" ${CONTAINER_USER}
USER ${CONTAINER_USER}
WORKDIR /home/${CONTAINER_USER}

# Install rustup
RUN curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
ENV PATH=${PATH}:/home/${CONTAINER_USER}/.cargo/bin

CMD [ "/bin/bash" ]
