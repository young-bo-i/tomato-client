#!/bin/bash
# ============================================================
# 本地构建 tomato-kol 服务端镜像并推送到阿里云 ACR
#
# 包含两个镜像:
#   - tomato-server  (Rust 服务端,/server 目录)
#   - tomato-abogus  (Node 签名服务,/server/abogusapp 目录)
#
# Postgres / Redis 用官方镜像,不在此处构建。
#
# 使用方法 (在 server/ 目录下运行):
#   ./scripts/build-and-push.sh           # 自动递增补丁版本 (0.0.1 -> 0.0.2)
#   ./scripts/build-and-push.sh patch     # 同上
#   ./scripts/build-and-push.sh minor     # 0.0.1 -> 0.1.0
#   ./scripts/build-and-push.sh major     # 0.0.1 -> 1.0.0
#   ./scripts/build-and-push.sh v1.2.3    # 使用指定版本号
#   ./scripts/build-and-push.sh patch server   # 只构建 server
#   ./scripts/build-and-push.sh patch abogus   # 只构建 abogus
# ============================================================

set -e

# ==================== 配置区域 ====================
REGISTRY="crpi-ebrftpuujezamyxv.cn-shanghai.personal.cr.aliyuncs.com"
NAMESPACE="cherrycans"
ALIYUN_USERNAME="aliyun6925902158"
VERSION_FILE="VERSION"
# 服务器架构 (阿里云 ECS 通常是 x86_64)
TARGET_PLATFORM="linux/amd64"
# ================================================

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# 检查是否在 server/ 目录
if [ ! -f "docker-compose.yml" ] || [ ! -d "abogusapp" ] || [ ! -f "Dockerfile" ]; then
    echo -e "${RED}错误: 请在 server/ 目录运行此脚本${NC}"
    exit 1
fi

# 检查并设置 buildx (Mac → Linux 需要跨架构构建)
setup_buildx() {
    if ! docker buildx version &> /dev/null; then
        echo -e "${RED}错误: docker buildx 不可用,请更新 Docker${NC}"
        exit 1
    fi

    if docker buildx inspect tomato-builder &> /dev/null; then
        docker buildx use tomato-builder
    else
        echo -e "${YELLOW}创建 buildx builder...${NC}"
        docker buildx create --name tomato-builder --driver docker-container --use
        docker buildx inspect --bootstrap
    fi
}

# 读取当前版本
get_current_version() {
    if [ -f "$VERSION_FILE" ]; then
        cat "$VERSION_FILE" | tr -d '[:space:]'
    else
        echo "0.0.0"
    fi
}

# 递增版本号
increment_version() {
    local version=$1
    local type=$2
    version=${version#v}

    local major=$(echo "$version" | cut -d. -f1)
    local minor=$(echo "$version" | cut -d. -f2)
    local patch=$(echo "$version" | cut -d. -f3)

    major=${major:-0}
    minor=${minor:-0}
    patch=${patch:-0}

    case $type in
        "major") major=$((major + 1)); minor=0; patch=0 ;;
        "minor") minor=$((minor + 1)); patch=0 ;;
        "patch"|*) patch=$((patch + 1)) ;;
    esac

    echo "${major}.${minor}.${patch}"
}

# 保存版本号
save_version() {
    echo "$1" > "$VERSION_FILE"
}

# 解析参数
VERSION_ARG=${1:-"patch"}
BUILD_TARGET=${2:-"all"}

if [[ "$VERSION_ARG" =~ ^v?[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    NEW_VERSION=${VERSION_ARG#v}
else
    CURRENT_VERSION=$(get_current_version)
    NEW_VERSION=$(increment_version "$CURRENT_VERSION" "$VERSION_ARG")
fi

# 镜像完整名称
SERVER_IMAGE="${REGISTRY}/${NAMESPACE}/tomato-server:${NEW_VERSION}"
SERVER_LATEST="${REGISTRY}/${NAMESPACE}/tomato-server:latest"
ABOGUS_IMAGE="${REGISTRY}/${NAMESPACE}/tomato-abogus:${NEW_VERSION}"
ABOGUS_LATEST="${REGISTRY}/${NAMESPACE}/tomato-abogus:latest"

echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  tomato-kol 服务端镜像构建推送工具${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo -e "当前版本: ${CYAN}$(get_current_version)${NC}"
echo -e "新版本:   ${YELLOW}${NEW_VERSION}${NC}"
echo -e "构建目标: ${YELLOW}${BUILD_TARGET}${NC}"
echo -e "目标架构: ${CYAN}${TARGET_PLATFORM}${NC}"
echo -e "仓库:     ${CYAN}${REGISTRY}/${NAMESPACE}${NC}"
echo ""

# 确认
read -p "确认构建并推送? [Y/n]: " CONFIRM
CONFIRM=${CONFIRM:-Y}
if [[ ! "$CONFIRM" =~ ^[Yy]$ ]]; then
    echo "已取消"
    exit 0
fi

# 登录阿里云 ACR
echo ""
echo -e "${YELLOW}=== 登录阿里云 ACR ===${NC}"
docker login --username=${ALIYUN_USERNAME} ${REGISTRY}

# 构建 tomato-server (Rust)
build_server() {
    echo ""
    echo -e "${YELLOW}=== 构建 tomato-server (${TARGET_PLATFORM}) ===${NC}"
    echo "版本镜像: ${SERVER_IMAGE}"
    echo "Latest:   ${SERVER_LATEST}"

    setup_buildx

    # buildx 一步完成跨架构构建 + push
    # 注意: Mac M 系列下用 qemu 模拟 amd64,Rust release 构建较慢 (~5-10min)
    docker buildx build \
        --platform ${TARGET_PLATFORM} \
        -t ${SERVER_IMAGE} \
        -t ${SERVER_LATEST} \
        --push \
        .

    echo -e "${GREEN}✓ tomato-server 完成${NC}"
}

# 构建 tomato-abogus (Node)
build_abogus() {
    echo ""
    echo -e "${YELLOW}=== 构建 tomato-abogus (${TARGET_PLATFORM}) ===${NC}"
    echo "版本镜像: ${ABOGUS_IMAGE}"
    echo "Latest:   ${ABOGUS_LATEST}"

    setup_buildx

    docker buildx build \
        --platform ${TARGET_PLATFORM} \
        -t ${ABOGUS_IMAGE} \
        -t ${ABOGUS_LATEST} \
        --push \
        ./abogusapp

    echo -e "${GREEN}✓ tomato-abogus 完成${NC}"
}

# 根据目标构建
case ${BUILD_TARGET} in
    "server"|"backend")
        build_server
        ;;
    "abogus")
        build_abogus
        ;;
    "all")
        build_server
        build_abogus
        ;;
    *)
        echo -e "${RED}错误: 未知的构建目标 '${BUILD_TARGET}'${NC}"
        echo "可用选项: server, abogus, all"
        exit 1
        ;;
esac

# 保存新版本号
save_version "${NEW_VERSION}"

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  构建推送完成!${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo -e "版本号已更新: ${YELLOW}${NEW_VERSION}${NC}"
echo ""
echo "已推送的镜像:"
if [ "${BUILD_TARGET}" = "all" ] || [ "${BUILD_TARGET}" = "server" ] || [ "${BUILD_TARGET}" = "backend" ]; then
    echo "  - ${SERVER_IMAGE}"
    echo "  - ${SERVER_LATEST}"
fi
if [ "${BUILD_TARGET}" = "all" ] || [ "${BUILD_TARGET}" = "abogus" ]; then
    echo "  - ${ABOGUS_IMAGE}"
    echo "  - ${ABOGUS_LATEST}"
fi
echo ""
echo "服务器部署: ./scripts/deploy.sh"
