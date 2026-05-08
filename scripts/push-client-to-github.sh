#!/bin/bash
# ============================================================
# 把 client/ 子目录单独同步到 GitHub 独立仓库
#
# 整个 monorepo 仍然主推 gitee。github 上只放 client/ 这个独立仓库,
# 用来跑 GitHub Actions 编译 Windows + Mac 客户端安装包。
#
# 使用方法 (在 monorepo 根目录运行):
#   ./scripts/push-client-to-github.sh                # 同步 client/ → github main
#   ./scripts/push-client-to-github.sh v0.0.1         # 同步 + 打 tag (触发 CI 构建)
#
# 触发 Actions 构建的两种方式:
#   A) 推 tag:  ./scripts/push-client-to-github.sh v0.0.1
#   B) 手动:    GitHub 仓库 → Actions → "Build Client" → "Run workflow"
# ============================================================

set -e

# ==================== 配置区域 ====================
REMOTE_NAME="github-client"
REMOTE_URL="https://github.com/young-bo-i/tomato-client.git"
PREFIX="client"
SOURCE_BRANCH="main"   # gitee monorepo 上的分支
TARGET_BRANCH="main"   # github 独立仓库上的分支
# ================================================

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

# 检查是否在 monorepo 根目录
if [ ! -d "$PREFIX" ] || [ ! -d ".git" ]; then
    echo -e "${RED}错误: 请在 monorepo 根目录运行 (期望存在 ${PREFIX}/ 和 .git/)${NC}"
    exit 1
fi

# 添加 github remote (如果还没添加)
if ! git remote get-url "$REMOTE_NAME" &>/dev/null; then
    echo -e "${YELLOW}添加远程仓库 ${REMOTE_NAME} → ${REMOTE_URL}${NC}"
    git remote add "$REMOTE_NAME" "$REMOTE_URL"
fi

VERSION_TAG="${1:-}"

# 如果传了版本号,先校验格式
if [[ -n "$VERSION_TAG" ]]; then
    if [[ ! "$VERSION_TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
        echo -e "${RED}错误: 版本号格式应为 vX.Y.Z (例: v0.0.1)${NC}"
        exit 1
    fi
fi

echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  client/ → GitHub 同步工具${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo -e "源:     ${CYAN}${SOURCE_BRANCH} 上的 ${PREFIX}/ 子目录${NC}"
echo -e "目标:   ${CYAN}${REMOTE_URL} → ${TARGET_BRANCH}${NC}"
[[ -n "$VERSION_TAG" ]] && echo -e "Tag:    ${YELLOW}${VERSION_TAG} (推送后会触发 Actions 构建)${NC}"
echo ""

# Step 1: subtree split — 生成一个把 client/ 当成根目录的合成提交链
echo -e "${YELLOW}[1/3] subtree split (把 client/ 提到根目录)...${NC}"
git branch -D _client_export 2>/dev/null || true
SPLIT_SHA=$(git subtree split --prefix="$PREFIX" "$SOURCE_BRANCH" -b _client_export)
echo "      合成 SHA: ${SPLIT_SHA:0:12}"

# Step 2: 推送到 github main
echo ""
echo -e "${YELLOW}[2/3] 推送 _client_export → ${REMOTE_NAME}/${TARGET_BRANCH}...${NC}"
# --force-with-lease: 安全的 force push,只有当远程分支没有意外变动时才会推
git push --force-with-lease "$REMOTE_NAME" "_client_export:${TARGET_BRANCH}"

# Step 3: 如果传了 tag,在合成 SHA 上打 tag 推上去 (触发 Actions)
if [[ -n "$VERSION_TAG" ]]; then
    echo ""
    echo -e "${YELLOW}[3/3] 推送 tag ${VERSION_TAG} → ${REMOTE_NAME}...${NC}"
    git push "$REMOTE_NAME" "${SPLIT_SHA}:refs/tags/${VERSION_TAG}"
else
    echo ""
    echo -e "${YELLOW}[3/3] 跳过 tag (未指定版本号)${NC}"
fi

# 清理临时分支
git branch -D _client_export 2>/dev/null || true

echo ""
echo -e "${GREEN}========================================${NC}"
echo -e "${GREEN}  ✓ 同步完成${NC}"
echo -e "${GREEN}========================================${NC}"
echo ""
echo -e "GitHub 仓库: ${CYAN}${REMOTE_URL}${NC}"
if [[ -n "$VERSION_TAG" ]]; then
    echo -e "已打 tag:    ${CYAN}${VERSION_TAG}${NC} — Actions 应该已开始构建 Win + Mac"
    echo -e "Actions:     ${CYAN}https://github.com/young-bo-i/tomato-client/actions${NC}"
else
    echo "提示:"
    echo "  - 想跑构建?用以下任一方式:"
    echo "    1) ./scripts/push-client-to-github.sh v0.0.1     (打 tag 自动触发)"
    echo "    2) GitHub Actions 页面手动点 Run workflow"
fi
