# This dockerfile serves as an example of a dockerfile that can be used with lelouch.
# Dockerfiles for agents must provide a comprehensive set of tools, similar to a developer's own machine, in order to be effective. Therefore, this file contains much more than one would expect from a normal dockerfile.

FROM ubuntu:26.04

# Install dependencies
RUN apt-get update && apt-get install -y \
    libsecret-1-0 \
    curl \
    wget \
    git \
    python3 \
    python3-pip \
    nodejs \
    npm \
    && rm -rf /var/lib/apt/lists/*


# Install common CLIs for agents

## Install Cursor Agent CLI
RUN curl -fsSL https://cursor.sh/install.sh | bash
## Install Gemini CLI
RUN npm install -g @google/gemini-cli

# Default working directory (lelouch will override this and mount the worktree to its absolute host path)
WORKDIR /workspace

# Default command (this will be overridden by lelouch)
CMD ["bash"]