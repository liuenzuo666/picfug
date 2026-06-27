# syntax=docker/dockerfile:1
# picfug 多阶段构建
#
# 阶段 1：在 rust 镜像里编译 release 二进制
# 阶段 2：把二进制拷进极小的运行镜像（distroless，无 shell，更安全）
#
# 构建支持多平台（amd64/arm64），可在任意 Linux 服务器运行。

# ============ 编译阶段 ============
FROM rust:1.90-bookworm AS builder

# 装构建依赖（如有额外系统库需求在此添加）
WORKDIR /build

# 先单独拷依赖清单，利用 Docker 层缓存：只有依赖变了才重新下载编译依赖
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    echo "pub fn placeholder() {}" > src/lib.rs && \
    cargo build --release && \
    rm -rf src target/release/deps/picfug* target/release/picfug

# 拷真实源码并编译
COPY . .
RUN touch src/main.rs src/lib.rs && cargo build --release

# ============ 运行阶段 ============
# distroless: 无 shell、无包管理器，镜像极小且攻击面小。
# 使用 nonroot 基础镜像，进程以 UID 65532 运行，避免容器内 root。
FROM gcr.io/distroless/cc-debian12:nonroot

LABEL org.opencontainers.image.title="picfug" \
      org.opencontainers.image.description="图片文件自动转换 CLI 守护进程" \
      org.opencontainers.image.source="https://github.com/liuenzuo/picfug"

WORKDIR /app

# 拷贝编译好的二进制
COPY --from=builder /build/target/release/picfug /usr/local/bin/picfug

# 默认数据卷（config.yaml / db / logs 在运行时挂载或在此初始化）
# /data 用于挂载宿主图片目录之外的持久化数据（db、logs、config）
VOLUME ["/data"]

# 以 nonroot 用户运行（distroless nonroot 镜像内置 USER 65532）
USER 65532:65532

ENTRYPOINT ["picfug"]
# 默认启动守护进程；可通过 docker run 覆盖为 once/check/status 等
CMD ["start", "-c", "/data/config.yaml"]
