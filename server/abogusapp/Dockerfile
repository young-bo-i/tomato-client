# 使用官方 Node.js 运行时作为父镜像
FROM node:18.17.1

# 设置工作目录
WORKDIR /usr/src/app

# 复制 package.json 和 package-lock.json
COPY package*.json ./

# 安装依赖
RUN npm install

# 复制项目文件
COPY . .

# 暴露端口
EXPOSE 3000

# 启动应用程序
CMD ["node", "a-bogus.js"]