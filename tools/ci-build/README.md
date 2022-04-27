ci-build
========

This directory includes the build Docker image and scripts to use it in CI.
- `../Dockerfile`: Dockerfile used to create the base build image. Needs to be in `tools/` root so that it
  can copy all the tools source code into the image at build time.
- `acquire-build-image`: Script that retrieves the base build image from public ECR or creates one locally
  depending on the state of the tools directory. If the git hash of the tools directory doesn't exist as a tag
  in public ECR, then it will build a new image locally. Otherwise, it will download the existing one and reuse it.
- `add-local-user.dockerfile`: Creates a user in the build image with the host's user ID
- `build.docker-compose.yml`: Docker Compose file for using the build image
- `ci-action`: Script for running CI actions inside of the Docker build image
- `ci-create-workspace`: Used by `ci-action`, but can be run manually to create a one-off workspace for debugging
- `scripts/`: CI scripts that get copied into the build image

There are three spaces you need to conceptualize for testing this locally:
- **Origin:** The original `smithy-rs` where you're iterating on CI scripts.
- **Starting space:** Directory with `smithy-rs` to run CI checks against. You have to create this. Conceptually,
  this is equivalent to the GitHub Actions working directory.
- **Action space:** Temporary directory maintained by `ci-action` where the CI checks actually run.

To create the starting space, do the following:

```bash
cd /path/to/my/starting-space
git clone https://github.com/awslabs/smithy-rs.git
# Optionally check out the revision you want to work with in the checked out smithy-rs.
# Just make sure you are in /path/to/my/starting-space (or whatever you called it) after.
```

Then you can test CI against that starting space by running:
```bash
$ORIGIN_PATH/tools/ci-build/ci-action <action> [args...]
```

The action names are the names of the scripts in `scripts/`, and `[args...]` get forwarded to those scripts.

__Note:__ `ci-action` does not rebuild the build image, so if you modified a script,
you need to run `./acquire-build-image` from the origin `tools/ci-build` path.