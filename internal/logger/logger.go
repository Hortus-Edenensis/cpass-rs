package logger

import (
	"fmt"
	"io"
	"log"
	"os"
	"path/filepath"
	"time"
)

// Logger represents a logger instance
type Logger struct {
	name   string
	logger *log.Logger
	file   *os.File
}

var (
	globalLogFile *os.File
	defaultLogger *log.Logger
)

// InitLogger initializes the logging system
func InitLogger(logPath string) error {
	// Create log directory if not exists
	if err := os.MkdirAll(logPath, 0755); err != nil {
		return fmt.Errorf("failed to create log directory: %w", err)
	}

	// Create log file with timestamp
	timestamp := time.Now().Format("2006-01-02_15-04-05")
	logFileName := filepath.Join(logPath, fmt.Sprintf("cpass_%s.log", timestamp))

	file, err := os.OpenFile(logFileName, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		return fmt.Errorf("failed to open log file: %w", err)
	}

	globalLogFile = file
	multiWriter := io.MultiWriter(os.Stdout, file)
	defaultLogger = log.New(multiWriter, "", log.LstdFlags)

	return nil
}

// New creates a new logger with a specific name
func New(name string) *Logger {
	var writer io.Writer
	if globalLogFile != nil {
		writer = io.MultiWriter(os.Stdout, globalLogFile)
	} else {
		writer = os.Stdout
	}

	return &Logger{
		name:   name,
		logger: log.New(writer, fmt.Sprintf("[%s] ", name), log.LstdFlags),
		file:   globalLogFile,
	}
}

// Info logs an informational message
func (l *Logger) Info(format string, v ...interface{}) {
	msg := fmt.Sprintf(format, v...)
	l.logger.Printf("[INFO] %s", msg)
}

// Warn logs a warning message
func (l *Logger) Warn(format string, v ...interface{}) {
	msg := fmt.Sprintf(format, v...)
	l.logger.Printf("[WARN] %s", msg)
}

// Error logs an error message
func (l *Logger) Error(format string, v ...interface{}) {
	msg := fmt.Sprintf(format, v...)
	l.logger.Printf("[ERROR] %s", msg)
}

// Debug logs a debug message
func (l *Logger) Debug(format string, v ...interface{}) {
	msg := fmt.Sprintf(format, v...)
	l.logger.Printf("[DEBUG] %s", msg)
}

// Fatal logs a fatal message and exits
func (l *Logger) Fatal(format string, v ...interface{}) {
	msg := fmt.Sprintf(format, v...)
	l.logger.Printf("[FATAL] %s", msg)
	os.Exit(1)
}

// Close closes the logger and its file handle
func (l *Logger) Close() error {
	if l.file != nil {
		return l.file.Close()
	}
	return nil
}

// CloseGlobalLogger closes the global log file
func CloseGlobalLogger() error {
	if globalLogFile != nil {
		return globalLogFile.Close()
	}
	return nil
}

// Info logs an informational message using the default logger
func Info(format string, v ...interface{}) {
	if defaultLogger != nil {
		defaultLogger.Printf("[INFO] "+format, v...)
	} else {
		log.Printf("[INFO] "+format, v...)
	}
}

// Warn logs a warning message using the default logger
func Warn(format string, v ...interface{}) {
	if defaultLogger != nil {
		defaultLogger.Printf("[WARN] "+format, v...)
	} else {
		log.Printf("[WARN] "+format, v...)
	}
}

// Error logs an error message using the default logger
func Error(format string, v ...interface{}) {
	if defaultLogger != nil {
		defaultLogger.Printf("[ERROR] "+format, v...)
	} else {
		log.Printf("[ERROR] "+format, v...)
	}
}

// Fatal logs a fatal message using the default logger and exits
func Fatal(format string, v ...interface{}) {
	if defaultLogger != nil {
		defaultLogger.Printf("[FATAL] "+format, v...)
	} else {
		log.Printf("[FATAL] "+format, v...)
	}
	os.Exit(1)
}
