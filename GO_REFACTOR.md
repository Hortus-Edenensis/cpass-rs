# Go 重构项目说明

本文档说明了项目从 Python 到 Go 的重构情况。

## 项目结构

```
.
├── cmd/
│   └── cpass/           # 主程序入口
│       └── main.go      # 应用程序主文件
├── internal/            # 内部包（不对外暴露）
│   ├── config/          # 配置管理
│   │   └── config.go
│   ├── dialog/          # 命令行交互
│   │   └── dialog.go
│   ├── logger/          # 日志系统
│   │   └── logger.go
│   └── utils/           # 工具函数
│       └── utils.go
├── pkg/                 # 公共包（可对外暴露）
│   ├── cxapi/           # ChaoXing API 客户端
│   │   └── api.go
│   └── resolver/        # 任务解析器
│       └── resolver.go
├── Dockerfile.go        # Go 版本的 Docker 配置
├── Makefile            # 构建脚本
├── go.mod              # Go 模块定义
└── README.md           # 项目说明文档
```

## 已实现的功能

### 1. 配置管理 (`internal/config`)
- ✅ YAML 配置文件加载
- ✅ 默认配置值
- ✅ 路径配置管理
- ✅ 任务配置（work, video, document, exam）

### 2. 日志系统 (`internal/logger`)
- ✅ 多级别日志（Info, Warn, Error, Debug, Fatal）
- ✅ 日志文件输出
- ✅ 控制台输出
- ✅ 时间戳记录

### 3. 工具函数 (`internal/utils`)
- ✅ Cookie 序列化/反序列化
- ✅ 会话数据保存/加载
- ✅ 手机号和姓名脱敏
- ✅ 人脸图片路径查找
- ✅ 版本管理

### 4. 命令行交互 (`internal/dialog`)
- ✅ Logo 显示
- ✅ 会话选择界面
- ✅ 登录界面
- ✅ 账号信息显示
- ✅ 课程选择界面
- ✅ 考试选择界面
- ✅ 确认提交界面

### 5. API 客户端基础框架 (`pkg/cxapi`)
- ✅ HTTP 客户端配置
- ✅ Session 管理
- ✅ Cookie 管理
- ✅ 数据结构定义（AccountInfo, Class, Chapter, ExamDTO 等）
- ⚠️ 具体 API 接口实现（待完成）

### 6. 任务解析器基础框架 (`pkg/resolver`)
- ✅ MediaPlayResolver 结构
- ✅ DocumentResolver 结构
- ✅ QuestionResolver 结构
- ✅ 搜索器接口定义
- ⚠️ 具体实现逻辑（待完成）

### 7. 主程序 (`cmd/cpass`)
- ✅ 程序初始化流程
- ✅ 配置加载
- ✅ 日志初始化
- ✅ 会话管理
- ✅ 登录流程
- ✅ 课程选择
- ⚠️ 任务执行逻辑（待完成）

### 8. 构建和部署
- ✅ Makefile 支持
- ✅ 跨平台编译
- ✅ Docker 支持
- ✅ 文档更新

## 需要进一步实现的功能

### 高优先级

#### 1. ChaoXing API 完整实现 (`pkg/cxapi`)
需要实现以下 API 调用：

```go
// 登录相关
func (api *ChaoXingAPI) Login(phone, password string) (*AccountInfo, error)
func (api *ChaoXingAPI) LoginByQRCode() (*AccountInfo, error)
func (api *ChaoXingAPI) GetAccountInfo() (*AccountInfo, error)

// 课程相关
func (api *ChaoXingAPI) FetchClasses() ([]Class, error)
func (api *ChaoXingAPI) FetchChapters(courseID, classID string) (*ChapterContainer, error)

// 任务点相关
func (api *ChaoXingAPI) FetchTaskPoint(chapterID string) ([]TaskPoint, error)
func (api *ChaoXingAPI) ReportVideoProgress(videoID, objectID string, progress int) error
func (api *ChaoXingAPI) ReportDocumentProgress(documentID, objectID string) error

// 考试相关
func (api *ChaoXingAPI) FetchExams(courseID, classID string) ([]ExamDTO, error)
func (api *ChaoXingAPI) StartExam(examID string) error
func (api *ChaoXingAPI) SubmitAnswer(examID, questionID, answer string) error
func (api *ChaoXingAPI) SubmitExam(examID string) error

// 人脸识别
func (api *ChaoXingAPI) FetchFaceImage() (string, error)
func (api *ChaoXingAPI) UploadFace(imagePath string) error

// 验证码处理
func (api *ChaoXingAPI) FetchCaptcha() ([]byte, error)
func (api *ChaoXingAPI) SubmitCaptcha(code string) error
```

参考 Python 实现：
- `cxapi/base.py` - 基础 HTTP 请求
- `cxapi/api.py` - API 接口
- `cxapi/session.py` - 会话管理
- `cxapi/captcha/` - 验证码处理
- `cxapi/face_detection.py` - 人脸识别

#### 2. 任务解析器实现 (`pkg/resolver`)

**视频播放模拟** (`MediaPlayResolver.Execute`)
```go
func (r *MediaPlayResolver) Execute() error {
    // 1. 获取视频时长
    // 2. 计算播放进度
    // 3. 按照 reportRate 定期上报进度
    // 4. 处理异常情况
}
```

**文档浏览模拟** (`DocumentResolver.Execute`)
```go
func (r *DocumentResolver) Execute() error {
    // 1. 获取文档信息
    // 2. 模拟浏览时间
    // 3. 上报完成状态
}
```

**题目搜索和答题** (`QuestionResolver.Execute`)
```go
func (r *QuestionResolver) Execute() error {
    // 1. 从题库后端搜索答案
    // 2. 匹配答案到选项
    // 3. 提交答案
    // 4. 处理未匹配情况（fallback）
}
```

参考 Python 实现：
- `resolver/media.py` - 视频播放
- `resolver/document.py` - 文档浏览
- `resolver/question.py` - 题目解答
- `resolver/searcher/` - 各种搜索器

#### 3. 题库搜索器实现

需要实现以下搜索器：

- **REST API 搜索器** (`RESTAPISearcher`)
- **JSON 文件搜索器** (`JSONSearcher`)
- **SQLite 数据库搜索器** (`SQLiteSearcher`)
- **ChatGPT/LLM 搜索器** (新增)

可能需要的依赖：
```go
import (
    "database/sql"
    _ "github.com/mattn/go-sqlite3"  // SQLite
    "github.com/tidwall/gjson"        // JSON 解析
)
```

#### 4. 验证码识别

Go 中实现 OCR 识别，可以考虑：
- 调用外部 Python 脚本（ddddocr）
- 使用 Go 的 OCR 库（如 gosseract）
- 调用在线 OCR 服务

### 中优先级

#### 5. 二维码登录实现
需要：
- 生成二维码（使用 `github.com/skip2/go-qrcode`）
- 轮询检查登录状态
- 获取登录凭证

#### 6. 命令解析器
实现课程选择语法解析：
- 支持序号：`0`, `1`, `2`
- 支持范围：`0-3`, `5-10`
- 支持课程名：`"解析几何"`
- 支持 courseId：`#23026xxx`
- 支持考试模式：`EXAM|0`

#### 7. TUI 界面
当前使用简单的命令行输出，可以考虑使用：
- `github.com/charmbracelet/bubbletea` - 现代 TUI 框架
- `github.com/gizak/termui` - 终端 UI 库
- `github.com/jroimartin/gocui` - Go Console UI

### 低优先级

#### 8. 性能优化
- 并发处理多个任务点
- 使用连接池
- 缓存机制

#### 9. 测试
- 单元测试
- 集成测试
- Mock API 服务器

#### 10. CI/CD
- GitHub Actions 工作流
- 自动构建多平台二进制
- Docker 镜像自动发布

## 开发建议

### 1. 开发顺序
建议按以下顺序开发：
1. 完成 ChaoXing API 的基础 HTTP 请求封装
2. 实现登录功能（手机号+密码）
3. 实现课程和章节获取
4. 实现视频播放模拟
5. 实现题库搜索器
6. 实现题目解答
7. 实现其他功能（二维码登录、验证码等）

### 2. 测试方法
- 使用真实的学习通账号测试（测试环境）
- 记录 HTTP 请求和响应，创建 Mock 数据
- 编写单元测试

### 3. 代码风格
遵循 Go 语言规范：
- 使用 `gofmt` 格式化代码
- 使用 `golint` 检查代码
- 遵循 effective Go 建议

### 4. 错误处理
Go 的错误处理模式：
```go
if err != nil {
    return fmt.Errorf("operation failed: %w", err)
}
```

### 5. 日志记录
使用已实现的 logger：
```go
logger := logger.New("ModuleName")
logger.Info("操作成功")
logger.Error("操作失败: %v", err)
```

## 依赖包建议

可能需要添加的依赖：

```bash
# HTTP 客户端增强
go get github.com/go-resty/resty/v2

# HTML 解析
go get github.com/PuerkitoBio/goquery

# JSON 处理
go get github.com/tidwall/gjson

# SQLite
go get github.com/mattn/go-sqlite3

# 二维码生成
go get github.com/skip2/go-qrcode

# TUI 框架
go get github.com/charmbracelet/bubbletea

# 命令行参数解析
go get github.com/spf13/cobra

# 进度条
go get github.com/schollz/progressbar/v3
```

## Python 代码参考

原 Python 代码保留在以下位置供参考：
- `main.py` - 主程序逻辑
- `cxapi/` - API 实现
- `resolver/` - 解析器实现
- `dialog.py` - 交互逻辑
- `logger.py` - 日志实现
- `utils.py` - 工具函数

## 构建和测试

```bash
# 编译
make build

# 运行
./cpass

# 清理
make clean

# 跨平台编译
make build-all

# Docker 构建
docker build -f Dockerfile.go -t cpass:go .
```

## 总结

当前 Go 重构已完成基础框架搭建，包括：
- ✅ 项目结构设计
- ✅ 配置管理
- ✅ 日志系统
- ✅ 工具函数
- ✅ 命令行交互
- ✅ 基础 API 框架
- ✅ 基础任务解析器框架

需要进一步实现的核心功能：
- ⚠️ ChaoXing API 的完整 HTTP 请求实现
- ⚠️ 任务点执行逻辑
- ⚠️ 题库搜索和答题逻辑
- ⚠️ 验证码和人脸识别

整体代码结构清晰，模块划分合理，为后续开发奠定了良好的基础。
