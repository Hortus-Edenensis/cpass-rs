package cxapi

import (
	"crypto/tls"
	"net/http"
	"time"
)

// ChaoXingAPI represents the ChaoXing API client
type ChaoXingAPI struct {
	client  *http.Client
	session *Session
	baseURL string
}

// Session represents a ChaoXing session
type Session struct {
	cookies map[string]string
	headers map[string]string
}

// NewChaoXingAPI creates a new ChaoXing API client
func NewChaoXingAPI() *ChaoXingAPI {
	// Create custom HTTP client with timeout and SSL configuration
	client := &http.Client{
		Timeout: 30 * time.Second,
		Transport: &http.Transport{
			TLSClientConfig: &tls.Config{
				InsecureSkipVerify: false,
			},
			MaxIdleConns:        100,
			MaxIdleConnsPerHost: 100,
			IdleConnTimeout:     90 * time.Second,
		},
	}

	return &ChaoXingAPI{
		client:  client,
		session: NewSession(),
		baseURL: "https://www.chaoxing.com",
	}
}

// NewSession creates a new session
func NewSession() *Session {
	return &Session{
		cookies: make(map[string]string),
		headers: map[string]string{
			"User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/91.0.4472.124 Safari/537.36",
		},
	}
}

// SetCookie sets a cookie in the session
func (s *Session) SetCookie(name, value string) {
	s.cookies[name] = value
}

// GetCookie gets a cookie from the session
func (s *Session) GetCookie(name string) string {
	return s.cookies[name]
}

// SetHeader sets a header in the session
func (s *Session) SetHeader(name, value string) {
	s.headers[name] = value
}

// LoadCookies loads cookies from a map
func (s *Session) LoadCookies(cookies map[string]string) {
	for k, v := range cookies {
		s.cookies[k] = v
	}
}

// AccountInfo represents user account information
type AccountInfo struct {
	Phone string `json:"phone"`
	PUID  int    `json:"puid"`
	Name  string `json:"name"`
}

// Login performs login with phone and password
func (api *ChaoXingAPI) Login(phone, password string) (*AccountInfo, error) {
	// TODO: Implement actual login logic
	// This is a placeholder implementation
	return &AccountInfo{
		Phone: phone,
		PUID:  0,
		Name:  "User",
	}, nil
}

// Session returns the current session
func (api *ChaoXingAPI) Session() *Session {
	return api.session
}

// GetAccountInfo gets current account information
func (api *ChaoXingAPI) GetAccountInfo() (*AccountInfo, error) {
	// TODO: Implement actual account info retrieval
	return &AccountInfo{}, nil
}

// FetchClasses fetches the list of classes
func (api *ChaoXingAPI) FetchClasses() ([]Class, error) {
	// TODO: Implement actual class fetching
	return []Class{}, nil
}

// Class represents a course/class
type Class struct {
	CourseID string `json:"course_id"`
	Name     string `json:"name"`
	ClassID  string `json:"class_id"`
}

// ChapterContainer represents a container for chapters
type ChapterContainer struct {
	Name     string    `json:"name"`
	Chapters []Chapter `json:"chapters"`
}

// Chapter represents a chapter in a course
type Chapter struct {
	ChapterID string `json:"chapter_id"`
	Name      string `json:"name"`
	Label     string `json:"label"`
}

// ExamDTO represents an exam data transfer object
type ExamDTO struct {
	ExamID     string `json:"exam_id"`
	Title      string `json:"title"`
	RemainTime int    `json:"remain_time"`
}

// PointVideoDTO represents a video task point
type PointVideoDTO struct {
	Title      string `json:"title"`
	VideoID    string `json:"video_id"`
	Duration   int    `json:"duration"`
	ObjectID   string `json:"object_id"`
}

// PointDocumentDTO represents a document task point
type PointDocumentDTO struct {
	Title      string `json:"title"`
	DocumentID string `json:"document_id"`
	ObjectID   string `json:"object_id"`
}

// PointWorkDTO represents a work/quiz task point
type PointWorkDTO struct {
	Title  string `json:"title"`
	WorkID string `json:"work_id"`
}
