from setuptools import setup

setup(
    name="librefang",
    version="2026.4.2b8",
    description="Official Python client for the LibreFang Agent OS REST API",
    py_modules=["librefang_sdk", "librefang_client"],
    python_requires=">=3.8",
    classifiers=[
        "Programming Language :: Python :: 3",
        "License :: OSI Approved :: MIT License",
        "Operating System :: OS Independent",
    ],
)
