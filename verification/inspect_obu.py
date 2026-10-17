import sys

# Let's inspect tests/intra_lossless.obu using the python-based parser or by reading it
data = open('tests/intra_lossless.obu', 'rb').read()
print(f"Total size: {len(data)}")

