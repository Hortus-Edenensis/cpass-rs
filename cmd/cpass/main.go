package main

import (
	"fmt"
	"os"
	"os/signal"
	"syscall"

	"github.com/theshdowaura/cpass/internal/config"
	"github.com/theshdowaura/cpass/internal/dialog"
	"github.com/theshdowaura/cpass/internal/logger"
	"github.com/theshdowaura/cpass/internal/utils"
	"github.com/theshdowaura/cpass/pkg/cxapi"
	"github.com/theshdowaura/cpass/pkg/resolver"
)

func main() {
	// Setup signal handling for graceful shutdown
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)

	go func() {
		<-sigChan
		fmt.Println("\n\n收到中断信号，正在退出...")
		logger.CloseGlobalLogger()
		os.Exit(0)
	}()

	// Show logo
	dialog.ShowLogo()

	// Load configuration
	cfg, err := config.LoadConfig("config.yml")
	if err != nil {
		fmt.Fprintf(os.Stderr, "Failed to load config: %v\n", err)
		os.Exit(1)
	}

	// Initialize logger
	if err := logger.InitLogger(cfg.GetLogPath()); err != nil {
		fmt.Fprintf(os.Stderr, "Failed to initialize logger: %v\n", err)
		os.Exit(1)
	}
	defer logger.CloseGlobalLogger()

	mainLogger := logger.New("Main")
	mainLogger.Info("\n-----*任务开始执行*-----")
	mainLogger.Info("Version: %s", utils.Version)

	// Load sessions
	sessions, err := utils.SessionsLoad(cfg.GetSessionPath())
	if err != nil {
		mainLogger.Error("Failed to load sessions: %v", err)
		os.Exit(1)
	}

	// Create API client
	api := cxapi.NewChaoXingAPI()

	// Handle session selection or login
	if len(sessions) > 0 {
		var selectedSession *utils.SessionModule
		
		if cfg.MultiSession {
			selectedSession, err = dialog.SelectSession(sessions)
			if err != nil {
				mainLogger.Error("Failed to select session: %v", err)
				os.Exit(1)
			}
		} else {
			selectedSession = &sessions[0]
		}

		// Load session cookies
		cookies := utils.CK2Dict(selectedSession.CK)
		api.Session().LoadCookies(cookies)

		// Verify session
		accInfo, err := api.GetAccountInfo()
		if err != nil || accInfo.PUID == 0 {
			fmt.Println("会话失效，请重新登录")
			// Perform re-login
			phone, password, useQRCode, err := dialog.Login()
			if err != nil {
				mainLogger.Error("Login failed: %v", err)
				os.Exit(1)
			}
			
			if useQRCode {
				fmt.Println("二维码登录功能尚未实现")
				os.Exit(1)
			}
			
			accInfo, err = api.Login(phone, password)
			if err != nil {
				mainLogger.Error("Login failed: %v", err)
				os.Exit(1)
			}
		}

		dialog.ShowAccountInfo(accInfo, cfg.MaskAcc)
	} else {
		fmt.Println("会话存档为空，请登录账号")
		phone, password, useQRCode, err := dialog.Login()
		if err != nil {
			mainLogger.Error("Login failed: %v", err)
			os.Exit(1)
		}
		
		if useQRCode {
			fmt.Println("二维码登录功能尚未实现")
			os.Exit(1)
		}
		
		accInfo, err := api.Login(phone, password)
		if err != nil {
			mainLogger.Error("Login failed: %v", err)
			os.Exit(1)
		}
		
		dialog.ShowAccountInfo(accInfo, cfg.MaskAcc)
	}

	// Fetch classes
	classes, err := api.FetchClasses()
	if err != nil {
		mainLogger.Error("Failed to fetch classes: %v", err)
		os.Exit(1)
	}

	if len(classes) == 0 {
		fmt.Println("未找到课程")
		return
	}

	// Select class
	command, err := dialog.SelectClass(classes)
	if err != nil {
		mainLogger.Error("Failed to select class: %v", err)
		os.Exit(1)
	}

	mainLogger.Info("Selected command: %s", command)

	// TODO: Parse command and execute tasks
	// This is where the task execution logic would go:
	// - Parse the command (handle EXAM| prefix, ranges, etc.)
	// - For each selected course:
	//   - If exam mode, fetch and execute exams
	//   - Otherwise, fetch chapters and execute task points
	
	fmt.Println("\n注意: 这是Go语言重构版本的基础框架")
	fmt.Println("完整的任务执行逻辑需要进一步实现以下模块:")
	fmt.Println("  - ChaoXing API的完整HTTP请求实现")
	fmt.Println("  - 视频播放模拟逻辑")
	fmt.Println("  - 文档浏览模拟逻辑")
	fmt.Println("  - 题目搜索和自动答题逻辑")
	fmt.Println("  - 验证码识别和人脸识别功能")
	fmt.Println("  - 考试模式和章节模式的完整工作流")

	// Example: Video task simulation
	if cfg.Video.Enable {
		mainLogger.Info("视频任务已启用 (速度: %.1fx, 上报间隔: %ds)", cfg.Video.Speed, cfg.Video.ReportRate)
	}

	// Example: Work task simulation
	if cfg.Work.Enable {
		mainLogger.Info("作业任务已启用 (导出: %v, 回退保存: %v)", cfg.Work.Export, cfg.Work.FallbackSave)
	}

	// Example: Document task simulation
	if cfg.Document.Enable {
		mainLogger.Info("文档任务已启用")
	}

	// Create example resolvers (not executed, just for demonstration)
	_ = resolver.NewMediaPlayResolver(nil, cfg.Video.Speed, cfg.Video.ReportRate)
	_ = resolver.NewDocumentResolver(nil)
	_ = resolver.NewQuestionResolver(nil, cfg.Work.FallbackSave, cfg.Work.FallbackFuzzer, cfg.Exam.PerSubmitDelay)

	mainLogger.Info("\n-----*任务执行完毕, 程序退出*-----")
	fmt.Println("\n任务已完成，程序退出")
}
