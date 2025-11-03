.PHONY: help install install-dev proto clean test test-unit test-integration test-e2e lint format type-check build docs

help:
	@echo "NoriKV Python SDK - Development Commands"
	@echo ""
	@echo "Setup:"
	@echo "  make install          Install runtime dependencies"
	@echo "  make install-dev      Install development dependencies"
	@echo ""
	@echo "Code Quality:"
	@echo "  make format           Format code with black"
	@echo "  make lint             Run ruff linter"
	@echo "  make type-check       Run mypy type checker"
	@echo ""
	@echo "Testing:"
	@echo "  make test             Run all tests"
	@echo "  make test-unit        Run unit tests only"
	@echo "  make test-integration Run integration tests (mock)"
	@echo "  make test-e2e         Run E2E tests (requires server)"
	@echo "  make test-cov         Run tests with coverage report"
	@echo ""
	@echo "Build:"
	@echo "  make proto            Generate protobuf code"
	@echo "  make build            Build distribution packages"
	@echo "  make clean            Clean generated files"

install:
	pip install -r requirements.txt

install-dev:
	pip install -r requirements-dev.txt
	pip install -e .

proto:
	@echo "Generating protobuf code..."
	python -m grpc_tools.protoc \
		-I../../proto \
		--python_out=norikv/proto \
		--grpc_python_out=norikv/proto \
		--mypy_out=norikv/proto \
		../../proto/norikv.proto
	@echo "Protobuf generation complete"

clean:
	rm -rf build/ dist/ *.egg-info/
	rm -rf .pytest_cache/ .mypy_cache/ .ruff_cache/
	rm -rf htmlcov/ .coverage
	find . -type d -name __pycache__ -exec rm -rf {} +
	find . -type f -name "*.pyc" -delete
	rm -rf norikv/proto/*_pb2.py norikv/proto/*_pb2.pyi norikv/proto/*_pb2_grpc.py

format:
	black norikv/ tests/ examples/

lint:
	ruff check norikv/ tests/ examples/

type-check:
	mypy norikv/ tests/

test:
	pytest tests/

test-unit:
	pytest tests/unit/ -m unit

test-integration:
	pytest tests/integration/ -m integration

test-e2e:
	pytest tests/integration/e2e/ -m e2e

test-cov:
	pytest tests/ --cov=norikv --cov-report=html --cov-report=term

build: clean proto
	python -m build

docs:
	@echo "Documentation generation not yet implemented"
