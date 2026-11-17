# Logs in to ChirpStack's API with gRPC-web: prints "ok" with a token, or the grpc-status.
import struct, sys, urllib.request
def field(n, s):
    b = s.encode(); return bytes([n << 3 | 2, len(b)]) + b
msg = field(1, sys.argv[1]) + field(2, sys.argv[2])
body = b"\x00" + struct.pack(">I", len(msg)) + msg
req = urllib.request.Request("http://127.0.0.1:8080/api.InternalService/Login", data=body,
    headers={"content-type": "application/grpc-web+proto", "x-grpc-web": "1"})
with urllib.request.urlopen(req, timeout=10) as r:
    status = r.headers.get("grpc-status")
    data = r.read()
if status is None:
    # The status is in the trailer frame at the end of the body.
    i = 0
    while i < len(data):
        flag, length = data[i], struct.unpack(">I", data[i+1:i+5])[0]
        frame = data[i+5:i+5+length]; i += 5 + length
        if flag & 0x80:
            for line in frame.decode().split("\r\n"):
                if line.lower().startswith("grpc-status:"): status = line.split(":",1)[1].strip()
print("ok" if status == "0" else f"refused (grpc-status {status})")
