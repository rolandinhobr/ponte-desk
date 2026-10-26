"""Print only the public certificate fingerprint, never the private key."""
import base64,hashlib,pathlib,re,sys
pem=pathlib.Path(sys.argv[1]).read_text()
match=re.search(r'-----BEGIN CERTIFICATE-----\s*(.*?)\s*-----END CERTIFICATE-----',pem,re.S)
if match is None:raise SystemExit('Certificate block not found')
digest=hashlib.sha256(base64.b64decode(match.group(1))).digest()
print(':'.join(f'{byte:02x}' for byte in digest))
