.PHONY: build clean run test install help

# Build variables
BINARY_NAME=cpass
BUILD_DIR=.
CMD_DIR=./cmd/cpass
GO=go

# Build the application
build:
	@echo "Building $(BINARY_NAME)..."
	$(GO) build -o $(BUILD_DIR)/$(BINARY_NAME) $(CMD_DIR)
	@echo "Build complete: $(BUILD_DIR)/$(BINARY_NAME)"

# Build for multiple platforms
build-all:
	@echo "Building for multiple platforms..."
	GOOS=linux GOARCH=amd64 $(GO) build -o $(BUILD_DIR)/$(BINARY_NAME)-linux-amd64 $(CMD_DIR)
	GOOS=linux GOARCH=arm64 $(GO) build -o $(BUILD_DIR)/$(BINARY_NAME)-linux-arm64 $(CMD_DIR)
	GOOS=windows GOARCH=amd64 $(GO) build -o $(BUILD_DIR)/$(BINARY_NAME)-windows-amd64.exe $(CMD_DIR)
	GOOS=darwin GOARCH=amd64 $(GO) build -o $(BUILD_DIR)/$(BINARY_NAME)-darwin-amd64 $(CMD_DIR)
	GOOS=darwin GOARCH=arm64 $(GO) build -o $(BUILD_DIR)/$(BINARY_NAME)-darwin-arm64 $(CMD_DIR)
	@echo "Cross-compilation complete"

# Clean build artifacts
clean:
	@echo "Cleaning..."
	rm -f $(BUILD_DIR)/$(BINARY_NAME)
	rm -f $(BUILD_DIR)/$(BINARY_NAME)-*
	@echo "Clean complete"

# Run the application
run: build
	@echo "Running $(BINARY_NAME)..."
	./$(BINARY_NAME)

# Run tests
test:
	@echo "Running tests..."
	$(GO) test -v ./...

# Install dependencies
deps:
	@echo "Downloading dependencies..."
	$(GO) mod download
	$(GO) mod tidy
	@echo "Dependencies installed"

# Format code
fmt:
	@echo "Formatting code..."
	$(GO) fmt ./...

# Run linter
lint:
	@echo "Running linter..."
	golangci-lint run ./...

# Display help
help:
	@echo "Available targets:"
	@echo "  build      - Build the application"
	@echo "  build-all  - Build for multiple platforms"
	@echo "  clean      - Remove build artifacts"
	@echo "  run        - Build and run the application"
	@echo "  test       - Run tests"
	@echo "  deps       - Download dependencies"
	@echo "  fmt        - Format code"
	@echo "  lint       - Run linter"
	@echo "  help       - Display this help message"
