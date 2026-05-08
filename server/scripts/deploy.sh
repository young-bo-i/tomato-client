#!/bin/bash
# ============================================================
# tomato-kol 服务端部署脚本 (在生产服务器上运行)
#
# 从阿里云 ACR 拉取镜像并启动服务。
#
# 使用方法 (在 server/ 目录下运行):
#   ./scripts/deploy.sh              # 拉取 latest 并部署
#   ./scripts/deploy.sh v1.0.1       # 拉取指定版本并部署
#   ./scripts/deploy.sh latest pull  # 只拉取不重启
#   ./scripts/deploy.sh latest restart # 只重启不拉取
#   ./scripts/deploy.sh latest status  # 查看服务状态
#
# 首次部署前请确保:
#   1. server/ 目录下存在 .env 文件 (至少包含 JWT_SECRET)
#   2. data/postgres / data/redis 目录有写入权限
# ============================================================

set -e

# ==================== 配置区域 ====================
# 服务器在阿里云 VPC 内可改用内网地址 (免公网流量、更快):
#   crpi-ebrftpuujezamyxv-vpc.cn-shanghai.personal.cr.aliyuncs.com
REGISTRY="crpi-ebrftpuujezamyxv.cn-shanghai.personal.cr.aliyuncs.com"
NAMESPACE="cherrycans"
ALIYUN_USERNAME="aliyun6925902158"
COMPOSE_FILE="docker-compose.prod.yml"
# ================================================

# 版本号 (默认 latest)
VERSION=${1:-"latest"}
# 操作类型: all (默认), pull, restart, status
ACTION=${2:-"all"}

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

# 镜像完整名称
SERVER_IMAGE="${REGISTRY}/${NAMESPACE}/tomato-server:${VERSION}"
ABOGUS_IMAGE="${REGISTRY}/${NAMESPACE}/tomato-abogus:${VERSION}"

# compose file 通过 ${VERSION} 变量决定拉哪个 tag
export VERSION

echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  tomato-kol 服务端部署工具${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo -e "版本: ${YELLOW}${VERSION}${NC}"
echo -e "操作: ${YELLOW}${ACTION}${NC}"
echo -e "仓库: ${YELLOW}${REGISTRY}/${NAMESPACE}${NC}"
echo ""

# 检查 docker-compose 文件
if [ ! -f "${COMPOSE_FILE}" ]; then
    echo -e "${RED}错误: 找不到 ${COMPOSE_FILE}${NC}"
    echo "请确保在 server/ 目录运行此脚本"
    exit 1
fi

# 检查 .env (生产环境必须有 JWT_SECRET)
if [ ! -f ".env" ] && [ "$ACTION" != "status" ]; then
    echo -e "${YELLOW}警告: 未找到 .env 文件,JWT_SECRET 等环境变量将使用默认值${NC}"
fi

# 登录阿里云 ACR
login_acr() {
    echo -e "${YELLOW}=== 登录阿里云 ACR ===${NC}"
    docker login --username=${ALIYUN_USERNAME} ${REGISTRY}
    echo ""
}

# 拉取镜像
pull_images() {
    echo -e "${YELLOW}=== 拉取镜像 ===${NC}"
    echo ""

    echo "拉取 tomato-server..."
    docker pull ${SERVER_IMAGE}
    echo ""

    echo "拉取 tomato-abogus..."
    docker pull ${ABOGUS_IMAGE}
    echo ""

    # postgres / redis 是官方镜像,compose up 时自动拉取
    echo -e "${GREEN}镜像拉取完成${NC}"
    echo ""
}

# 重启服务
restart_services() {
    echo -e "${YELLOW}=== 停止旧服务 ===${NC}"
    docker compose -f ${COMPOSE_FILE} down
    echo ""

    echo -e "${YELLOW}=== 启动新服务 ===${NC}"
    docker compose -f ${COMPOSE_FILE} up -d
    echo ""
}

# 清理悬挂镜像 (旧版本残留)
cleanup_images() {
    echo -e "${YELLOW}=== 清理未使用的镜像 ===${NC}"
    docker image prune -f
    echo ""
}

# 显示状态
show_status() {
    echo -e "${YELLOW}=== 服务状态 ===${NC}"
    docker compose -f ${COMPOSE_FILE} ps
    echo ""
}

# 主流程
case ${ACTION} in
    "all")
        login_acr
        pull_images
        restart_services
        cleanup_images
        show_status
        ;;
    "pull")
        login_acr
        pull_images
        ;;
    "restart")
        restart_services
        show_status
        ;;
    "status")
        show_status
        ;;
    *)
        echo -e "${RED}错误: 未知的操作 '${ACTION}'${NC}"
        echo "可用选项: all (默认), pull, restart, status"
        exit 1
        ;;
esac

echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  完成!${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo "常用命令:"
echo "  docker logs -f tomato-server"
echo "  docker logs -f tomato-abogus"
echo "  docker compose -f ${COMPOSE_FILE} logs -f"
echo "  docker compose -f ${COMPOSE_FILE} ps"
