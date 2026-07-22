.PHONY: help install check test build dev clean

# Display help information
help:
	@echo "LazyDNS WebUI Makefile"
	@echo ""
	@echo "Available targets:"
	@echo "  help       - Display this help message"
	@echo "  install    - Install dependencies (npm install + coverage tools)"
	@echo "  check      - Run TypeScript type checking (svelte-check)"
	@echo "  test       - Run unit tests (vitest)"
	@echo "  coverage   - Run unit tests with coverage report"
	@echo "  build      - Build for production"
	@echo "  dev        - Start development server (Vite)"
	@echo "  clean      - Clean build artifacts and dependencies"
	@echo "  lint       - Run linting and type checking"
	@echo ""

# Install dependencies
install:
	@echo "Installing dependencies..."
	npm install

# Run TypeScript type checking
check:
	@echo "Running TypeScript type checking..."
	npm run check

# Run unit tests
test:
	@echo "Running unit tests..."
	npm test

# Run unit tests with coverage
coverage:
	@echo "Running unit tests with coverage..."
	npm run test:coverage

# Build for production
build: check
	@echo "Building for production..."
	npm run build

# Start development server
dev:
	@echo "Starting development server..."
	npm run dev

# Clean build artifacts and dependencies
clean:
	@echo "Cleaning build artifacts..."
	rm -rf dist/
	rm -rf node_modules/
	rm -rf coverage/
	@echo "Clean complete"

# Run linting and type checking
lint: check
	@echo "Linting and type checking complete"

# Build and run development server
dev-full: install dev

# Full build pipeline
all: clean install lint test build
	@echo "Full build pipeline complete!"
