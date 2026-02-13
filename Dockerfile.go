# Build stage
FROM golang:1.21-alpine AS builder

# Install build dependencies
RUN apk add --no-cache git make

WORKDIR /build

# Copy go mod files
COPY go.mod go.sum ./
RUN go mod download

# Copy source code
COPY . .

# Build the application
RUN CGO_ENABLED=0 GOOS=linux go build -a -installsuffix cgo -o cpass ./cmd/cpass

# Runtime stage
FROM alpine:latest

ENV TZ="Asia/Shanghai"

# Install runtime dependencies
RUN apk --no-cache add ca-certificates tzdata

WORKDIR /app

# Copy binary from builder
COPY --from=builder /build/cpass /app/cpass

# Copy config file template (will be overridden by volume mount)
COPY config.yml /app/config.yml.example

# Create necessary directories
RUN mkdir -p /app/session /app/logs /app/export /app/faces

# Set permissions
RUN chmod +x /app/cpass

ENTRYPOINT ["/app/cpass"]
