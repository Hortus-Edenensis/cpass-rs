# 快速开始指南

本文档将帮助您快速上手使用 Go 版本的超星学习通答题姬。

## 前置要求

- Go 1.21 或更高版本
- 一个超星学习通账号
- `config.yml` 配置文件

## 安装

### 方式一：从源码构建

```bash
# 克隆仓库
git clone https://github.com/theshdowaura/cpass.git
cd cpass

# 编译
make build

# 运行
./cpass
```

### 方式二：直接下载二进制文件

从 [Releases](https://github.com/theshdowaura/cpass/releases) 页面下载适合您操作系统的二进制文件。

- Windows: `cpass-windows-amd64.exe`
- Linux: `cpass-linux-amd64`
- macOS (Intel): `cpass-darwin-amd64`
- macOS (Apple Silicon): `cpass-darwin-arm64`

下载后，给予执行权限（Linux/macOS）：
```bash
chmod +x cpass-linux-amd64
```

### 方式三：使用 Docker

```bash
# 构建镜像
docker build -f Dockerfile.go -t cpass:go .

# 运行容器
docker run -it \
  --name cpass \
  -v "$PWD/session:/app/session" \
  -v "$PWD/export:/app/export" \
  -v "$PWD/logs:/app/logs" \
  -v "$PWD/faces:/app/faces" \
  -v "$PWD/config.yml:/app/config.yml" \
  cpass:go
```

## 配置

### 1. 创建配置文件

复制 `config.yml` 并根据需要修改：

```yaml
# 路径配置
session_path: "session"      # 会话存档目录
log_path: "logs"             # 日志目录
export_path: "export"        # 导出目录
face_image_path: "faces"     # 人脸图片目录

# 基本配置
multi_session: true          # 是否支持多账号
tui_max_height: 25           # 终端 UI 最大高度
mask_acc: true               # 是否脱敏显示账号信息
fetch_uploaded_face: true    # 是否自动拉取已上传的人脸图片

# 任务配置
work:                        # 章节测验配置
  enable: true               # 是否启用
  wait: 15                   # 完成后等待时间（秒）
  export: false              # 是否导出试题
  fallback_save: false       # 答案匹配失败时是否临时保存
  fallback_fuzzer: false     # 答案匹配失败时是否随机填充

video:                       # 视频任务配置
  enable: true               # 是否启用
  wait: 15                   # 完成后等待时间（秒）
  speed: 1.0                 # 播放速度倍率
  report_rate: 60            # 进度上报间隔（秒）

document:                    # 文档任务配置
  enable: true               # 是否启用
  wait: 15                   # 完成后等待时间（秒）

exam:                        # 考试配置
  fallback_fuzzer: false     # 答案匹配失败时是否随机填充
  confirm_submit: true       # 提交前是否确认
  persubmit_delay: 5         # 每题提交延迟（秒）

# 题库搜索器配置
searchers: []                # 题库后端配置（待实现）
```

### 2. 创建必要的目录

```bash
mkdir -p session logs export faces
```

## 使用

### 基本使用流程

1. **启动程序**
   ```bash
   ./cpass
   ```

2. **登录账号**
   - 首次使用需要登录
   - 输入手机号和密码
   - 或直接回车使用二维码登录（功能开发中）

3. **选择会话**
   - 如果已有会话存档，可以选择使用
   - 支持多账号管理

4. **选择课程**
   - 程序会显示您的所有课程
   - 输入序号选择要学习的课程
   - 支持多个课程选择（用逗号分隔）

5. **自动执行任务**
   - 程序会自动执行视频、文档、测验等任务
   - 进度会显示在屏幕上并记录到日志文件

### 高级用法

#### 课程选择语法

```bash
# 单个课程（序号）
0

# 多个课程
0,1,2

# 课程范围
0-3

# 按课程名选择
"解析几何"

# 按 courseId 选择
#23026xxx

# 混合使用
0,1-3,"解析几何"
```

#### 考试模式

在课程选择时，在课程前加上 `EXAM|` 前缀进入考试模式：

```bash
# 对课程 0 进入考试模式
EXAM|0

# 对课程名进入考试模式
EXAM|"解析几何"
```

进入考试模式后，可以：
- 选择要参加的考试
- 导出试卷（输入 `e` + 序号）

#### 导出试卷

```bash
# 在考试选择界面，输入 e + 序号
e0  # 导出第 0 号考试的试卷
```

## 目录结构说明

```
cpass/
├── session/          # 会话存档（自动生成）
│   └── 1380000xxxx.json
├── logs/             # 日志文件（自动生成）
│   └── cpass_2024-01-01_12-00-00.log
├── export/           # 导出的试卷（自动生成）
│   └── exam_12345.json
├── faces/            # 人脸图片
│   └── 114514.jpg
├── config.yml        # 配置文件
└── cpass            # 可执行文件
```

## 常见问题

### 1. 会话失效怎么办？

程序会自动检测会话是否有效，如果失效会提示重新登录。

### 2. 如何查看详细日志？

所有操作都会记录到 `logs/` 目录下的日志文件中。

### 3. 支持哪些题库？

当前基础框架已完成，具体题库支持（REST API、JSON、SQLite、ChatGPT 等）正在开发中。

### 4. 程序运行出错怎么办？

1. 查看日志文件了解详细错误信息
2. 检查配置文件是否正确
3. 确保网络连接正常
4. 在 GitHub 提交 Issue

### 5. 如何更新程序？

```bash
# 从源码更新
git pull
make clean
make build

# 或下载最新的二进制文件
```

## 注意事项

⚠️ **重要提示**：

1. 本项目仅供学习研究使用
2. 请勿用于商业用途
3. 使用本项目产生的任何后果由使用者自行承担
4. 建议在使用前备份重要数据
5. 当前 Go 版本是基础框架，部分功能仍在开发中

## 开发状态

✅ **已完成**：
- 基础框架
- 配置管理
- 日志系统
- 会话管理
- 命令行交互

⚠️ **开发中**：
- ChaoXing API 完整实现
- 视频播放模拟
- 题目自动答题
- 验证码识别
- 人脸识别
- 二维码登录

详见 [GO_REFACTOR.md](GO_REFACTOR.md) 了解完整的开发计划。

## 获取帮助

- 查看 [README.md](README.md) 了解项目详情
- 查看 [GO_REFACTOR.md](GO_REFACTOR.md) 了解重构说明
- 提交 [Issue](https://github.com/theshdowaura/cpass/issues) 报告问题
- 参考 Python 原版代码了解实现细节

## 贡献

欢迎提交 PR 贡献代码！请确保：
- 代码符合 Go 语言规范
- 添加必要的注释
- 更新相关文档

## 许可证

本项目采用 GPL-3.0 许可证。详见 [LICENSE](LICENSE) 文件。
