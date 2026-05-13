from setuptools import setup

setup(
    name="bossfang-sdk",
    version="2026.5.12b11",
    description="Official Python client for the BossFang Agent OS REST API",
    # BossFang fork: PyPI package renamed to `bossfang-sdk` but the Python
    # module names stay `librefang_sdk` / `librefang_client` to match the
    # existing import paths in user code. Internal symbol names are
    # Layer-Internal — not part of the rebrand surface.
    py_modules=["librefang_sdk", "librefang_client"],
    python_requires=">=3.8",
    classifiers=[
        "Programming Language :: Python :: 3",
        "License :: OSI Approved :: MIT License",
        "Operating System :: OS Independent",
    ],
)
