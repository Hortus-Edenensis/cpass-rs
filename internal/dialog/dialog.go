package dialog

import (
	"bufio"
	"fmt"
	"os"
	"strings"

	"github.com/theshdowaura/cpass/internal/utils"
	"github.com/theshdowaura/cpass/pkg/cxapi"
)

// ShowLogo displays the application logo
func ShowLogo() {
	logo := `
   ____ ____   __    ____ ____  
  / ___/ ___| / /   |  _ \/ ___| 
 | |  | |    / /    | |_) \___ \ 
 | |__| |   / /     |  __/ ___) |
  \___\___|/_/      |_|   |____/ 
                                 
  超星学习通答题姬 (Go版本)
  CxPass - ChaoXing Auto Study Tool
  Version: %s
  =====================================
`
	fmt.Printf(logo, utils.Version)
}

// SelectSession prompts user to select a session
func SelectSession(sessions []utils.SessionModule) (*utils.SessionModule, error) {
	if len(sessions) == 0 {
		return nil, fmt.Errorf("no sessions available")
	}

	fmt.Println("\n请选择会话存档:")
	for i, sess := range sessions {
		phone := sess.Phone
		name := sess.Name
		fmt.Printf("[%d] %s - %s\n", i, phone, name)
	}

	reader := bufio.NewReader(os.Stdin)
	fmt.Print("\n输入序号: ")
	input, _ := reader.ReadString('\n')
	input = strings.TrimSpace(input)

	var idx int
	_, err := fmt.Sscanf(input, "%d", &idx)
	if err != nil || idx < 0 || idx >= len(sessions) {
		return nil, fmt.Errorf("invalid selection")
	}

	return &sessions[idx], nil
}

// Login prompts user to login
func Login() (phone, password string, useQRCode bool, err error) {
	reader := bufio.NewReader(os.Stdin)
	
	fmt.Print("\n请输入手机号 (直接回车使用二维码登录): ")
	phone, _ = reader.ReadString('\n')
	phone = strings.TrimSpace(phone)

	if phone == "" {
		return "", "", true, nil
	}

	fmt.Print("请输入密码: ")
	password, _ = reader.ReadString('\n')
	password = strings.TrimSpace(password)

	return phone, password, false, nil
}

// ShowAccountInfo displays account information
func ShowAccountInfo(acc *cxapi.AccountInfo, maskAcc bool) {
	fmt.Println("\n=== 账号信息 ===")
	if maskAcc {
		fmt.Printf("手机号: %s\n", utils.MaskPhone(acc.Phone))
		fmt.Printf("姓名: %s\n", utils.MaskName(acc.Name))
	} else {
		fmt.Printf("手机号: %s\n", acc.Phone)
		fmt.Printf("姓名: %s\n", acc.Name)
	}
	fmt.Printf("PUID: %d\n", acc.PUID)
	fmt.Println("================")
}

// SelectClass prompts user to select classes
func SelectClass(classes []cxapi.Class) (string, error) {
	if len(classes) == 0 {
		return "", fmt.Errorf("no classes available")
	}

	fmt.Println("\n=== 课程列表 ===")
	for i, class := range classes {
		fmt.Printf("[%d] %s (ID: %s)\n", i, class.Name, class.CourseID)
	}
	fmt.Println("================")

	reader := bufio.NewReader(os.Stdin)
	fmt.Print("\n请选择课程 (输入序号，多个用逗号分隔，输入 EXAM|序号 进入考试模式): ")
	input, _ := reader.ReadString('\n')
	input = strings.TrimSpace(input)

	return input, nil
}

// SelectExam prompts user to select an exam
func SelectExam(exams []cxapi.ExamDTO) (int, bool, error) {
	if len(exams) == 0 {
		return 0, false, fmt.Errorf("no exams available")
	}

	fmt.Println("\n=== 考试列表 ===")
	for i, exam := range exams {
		fmt.Printf("[%d] %s (ID: %s)\n", i, exam.Title, exam.ExamID)
		fmt.Printf("    e%d - 导出试卷\n", i)
	}
	fmt.Println("================")

	reader := bufio.NewReader(os.Stdin)
	fmt.Print("\n请选择考试 (输入序号，输入 e+序号 导出试卷): ")
	input, _ := reader.ReadString('\n')
	input = strings.TrimSpace(input)

	export := false
	if strings.HasPrefix(input, "e") {
		export = true
		input = strings.TrimPrefix(input, "e")
	}

	var idx int
	_, err := fmt.Sscanf(input, "%d", &idx)
	if err != nil || idx < 0 || idx >= len(exams) {
		return 0, false, fmt.Errorf("invalid selection")
	}

	return idx, export, nil
}

// ConfirmSubmit prompts user to confirm exam submission
func ConfirmSubmit(completedCnt, incompletedCnt int) bool {
	reader := bufio.NewReader(os.Stdin)
	fmt.Printf("\n答题完毕，完成 %d 题，未完成 %d 题\n", completedCnt, incompletedCnt)
	fmt.Print("请确认是否立即交卷 (y/n, 默认y): ")
	
	input, _ := reader.ReadString('\n')
	input = strings.TrimSpace(strings.ToLower(input))
	
	return input == "" || input == "y" || input == "yes"
}

// Prompt displays a prompt and waits for user input
func Prompt(message string) string {
	reader := bufio.NewReader(os.Stdin)
	fmt.Print(message)
	input, _ := reader.ReadString('\n')
	return strings.TrimSpace(input)
}
