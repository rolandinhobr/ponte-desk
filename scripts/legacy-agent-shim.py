"""Short-lived compatibility entry point; all policy lives in Ponte Desk."""
import os
from pathlib import Path
import sys
if sys.platform == 'win32':
    executable=Path(os.environ['LOCALAPPDATA'])/'Programs/PonteDesk/ponte-desk.exe'
else:
    executable=Path('/Applications/Ponte Desk.app/Contents/MacOS/ponte-desk')
verb='agent-hook' if 'hook' in Path(__file__).stem else 'agent'
os.execv(str(executable),[str(executable),verb,*sys.argv[1:]])
