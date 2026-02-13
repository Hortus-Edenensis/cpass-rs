package utils

import (
	"encoding/json"
	"fmt"
	"math/rand"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"
)

const Version = "0.4.5-go"

// SessionModule represents session data model
type SessionModule struct {
	Phone  string  `json:"phone"`
	PUID   int     `json:"puid"`
	Passwd *string `json:"passwd,omitempty"`
	Name   string  `json:"name"`
	CK     string  `json:"ck"`
}

// Dict2CK serializes dict-form cookie to string
func Dict2CK(dictCK map[string]string) string {
	var sb strings.Builder
	for k, v := range dictCK {
		sb.WriteString(fmt.Sprintf("%s=%s;", k, v))
	}
	return sb.String()
}

// CK2Dict parses cookie string to dict
func CK2Dict(ck string) map[string]string {
	result := make(map[string]string)
	fields := strings.Split(strings.TrimSpace(ck), ";")
	for _, field := range fields {
		if field == "" {
			continue
		}
		parts := strings.SplitN(field, "=", 2)
		if len(parts) == 2 {
			result[parts[0]] = parts[1]
		}
	}
	return result
}

// SaveSession saves session data as JSON
func SaveSession(sessionsPath, phone string, ck map[string]string, puid int, name string, passwd *string) error {
	if err := os.MkdirAll(sessionsPath, 0755); err != nil {
		return err
	}
	
	filePath := filepath.Join(sessionsPath, fmt.Sprintf("%s.json", phone))
	sessData := SessionModule{
		Phone:  phone,
		PUID:   puid,
		Passwd: passwd,
		Name:   name,
		CK:     Dict2CK(ck),
	}
	
	data, err := json.MarshalIndent(sessData, "", "  ")
	if err != nil {
		return err
	}
	
	return os.WriteFile(filePath, data, 0644)
}

// SessionsLoad loads sessions from path
func SessionsLoad(sessionsPath string) ([]SessionModule, error) {
	var sessions []SessionModule
	
	entries, err := os.ReadDir(sessionsPath)
	if err != nil {
		if os.IsNotExist(err) {
			return sessions, nil
		}
		return nil, err
	}
	
	for _, entry := range entries {
		if entry.IsDir() || filepath.Ext(entry.Name()) != ".json" {
			continue
		}
		
		filePath := filepath.Join(sessionsPath, entry.Name())
		data, err := os.ReadFile(filePath)
		if err != nil {
			continue
		}
		
		var sessData SessionModule
		if err := json.Unmarshal(data, &sessData); err != nil {
			continue
		}
		
		sessions = append(sessions, sessData)
	}
	
	return sessions, nil
}

// MaskName masks a name
func MaskName(name string) string {
	runes := []rune(name)
	length := len(runes)
	
	if length <= 2 {
		return string(runes[0]) + "*"
	}
	
	masked := string(runes[0])
	for i := 1; i < length-1; i++ {
		masked += "*"
	}
	masked += string(runes[length-1])
	
	return masked
}

// MaskPhone masks a phone number (must be 11 digits)
func MaskPhone(phone string) string {
	if len(phone) != 11 {
		return phone
	}
	return phone[:3] + "****" + phone[len(phone)-4:]
}

// GetFacePathByPUID gets and randomly selects face image path by puid
func GetFacePathByPUID(facePath string, puid int) (string, error) {
	pattern := filepath.Join(facePath, fmt.Sprintf("%d*.jpg", puid))
	matches, err := filepath.Glob(pattern)
	if err != nil {
		return "", err
	}
	
	var matchedImages []string
	re := regexp.MustCompile(`^\d+(_\d+)?$`)
	
	for _, match := range matches {
		base := filepath.Base(match)
		stem := strings.TrimSuffix(base, filepath.Ext(base))
		if re.MatchString(stem) {
			matchedImages = append(matchedImages, match)
		}
	}
	
	if len(matchedImages) == 0 {
		return "", fmt.Errorf("no face image found for puid %d", puid)
	}
	
	rand.Seed(time.Now().UnixNano())
	return matchedImages[rand.Intn(len(matchedImages))], nil
}
