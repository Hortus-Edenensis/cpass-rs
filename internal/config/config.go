package config

import (
	"fmt"
	"os"
	"path/filepath"

	"gopkg.in/yaml.v3"
)

// Config represents the application configuration
type Config struct {
	// Path configurations
	SessionPath    string `yaml:"session_path"`
	LogPath        string `yaml:"log_path"`
	ExportPath     string `yaml:"export_path"`
	FaceImagePath  string `yaml:"face_image_path"`

	// Basic configurations
	MultiSession      bool `yaml:"multi_session"`
	TUIMaxHeight      int  `yaml:"tui_max_height"`
	MaskAcc           bool `yaml:"mask_acc"`
	FetchUploadedFace bool `yaml:"fetch_uploaded_face"`

	// Task configurations
	Work     WorkConfig     `yaml:"work"`
	Video    VideoConfig    `yaml:"video"`
	Document DocumentConfig `yaml:"document"`
	Exam     ExamConfig     `yaml:"exam"`

	// Searcher configurations
	Searchers []map[string]interface{} `yaml:"searchers"`
}

// WorkConfig represents work task configuration
type WorkConfig struct {
	Enable          bool `yaml:"enable"`
	Wait            int  `yaml:"wait"`
	Export          bool `yaml:"export"`
	FallbackSave    bool `yaml:"fallback_save"`
	FallbackFuzzer  bool `yaml:"fallback_fuzzer"`
}

// VideoConfig represents video task configuration
type VideoConfig struct {
	Enable     bool    `yaml:"enable"`
	Wait       int     `yaml:"wait"`
	Speed      float64 `yaml:"speed"`
	ReportRate int     `yaml:"report_rate"`
}

// DocumentConfig represents document task configuration
type DocumentConfig struct {
	Enable bool `yaml:"enable"`
	Wait   int  `yaml:"wait"`
}

// ExamConfig represents exam task configuration
type ExamConfig struct {
	FallbackFuzzer  bool `yaml:"fallback_fuzzer"`
	ConfirmSubmit   bool `yaml:"confirm_submit"`
	PerSubmitDelay  int  `yaml:"persubmit_delay"`
}

var AppConfig *Config

// LoadConfig loads configuration from YAML file
func LoadConfig(configPath string) (*Config, error) {
	cfg := &Config{
		// Default values
		SessionPath:       "session",
		LogPath:           "logs",
		ExportPath:        "export",
		FaceImagePath:     "faces",
		MultiSession:      true,
		TUIMaxHeight:      25,
		MaskAcc:           true,
		FetchUploadedFace: true,
		Work: WorkConfig{
			Enable:         true,
			Wait:           15,
			Export:         false,
			FallbackSave:   false,
			FallbackFuzzer: false,
		},
		Video: VideoConfig{
			Enable:     true,
			Wait:       15,
			Speed:      1.0,
			ReportRate: 60,
		},
		Document: DocumentConfig{
			Enable: true,
			Wait:   15,
		},
		Exam: ExamConfig{
			FallbackFuzzer: false,
			ConfirmSubmit:  true,
			PerSubmitDelay: 5,
		},
	}

	data, err := os.ReadFile(configPath)
	if err != nil {
		if os.IsNotExist(err) {
			fmt.Fprintf(os.Stderr, "Warning: Config file not found, using defaults\n")
			AppConfig = cfg
			return cfg, nil
		}
		return nil, fmt.Errorf("failed to read config file: %w", err)
	}

	if err := yaml.Unmarshal(data, cfg); err != nil {
		return nil, fmt.Errorf("failed to parse config file: %w", err)
	}

	// Create necessary directories
	if err := os.MkdirAll(cfg.ExportPath, 0755); err != nil {
		return nil, fmt.Errorf("failed to create export directory: %w", err)
	}
	
	if err := os.MkdirAll(cfg.SessionPath, 0755); err != nil {
		return nil, fmt.Errorf("failed to create session directory: %w", err)
	}
	
	if err := os.MkdirAll(cfg.LogPath, 0755); err != nil {
		return nil, fmt.Errorf("failed to create log directory: %w", err)
	}

	// Store as global config
	AppConfig = cfg
	
	return cfg, nil
}

// GetSessionPath returns the absolute path to session directory
func (c *Config) GetSessionPath() string {
	if filepath.IsAbs(c.SessionPath) {
		return c.SessionPath
	}
	abs, _ := filepath.Abs(c.SessionPath)
	return abs
}

// GetLogPath returns the absolute path to log directory
func (c *Config) GetLogPath() string {
	if filepath.IsAbs(c.LogPath) {
		return c.LogPath
	}
	abs, _ := filepath.Abs(c.LogPath)
	return abs
}

// GetExportPath returns the absolute path to export directory
func (c *Config) GetExportPath() string {
	if filepath.IsAbs(c.ExportPath) {
		return c.ExportPath
	}
	abs, _ := filepath.Abs(c.ExportPath)
	return abs
}

// GetFaceImagePath returns the absolute path to face image directory
func (c *Config) GetFaceImagePath() string {
	if filepath.IsAbs(c.FaceImagePath) {
		return c.FaceImagePath
	}
	abs, _ := filepath.Abs(c.FaceImagePath)
	return abs
}
