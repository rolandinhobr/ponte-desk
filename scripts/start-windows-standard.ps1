param([Parameter(Mandatory=$true)][string]$Exe,[string]$Arguments='--capture-backend windows --emulation-backend windows daemon')
$ErrorActionPreference='Stop'
if(-not ('PonteDeskStandardBootstrap' -as [type])){
Add-Type -TypeDefinition @'
using System;using System.Text;using System.ComponentModel;using System.Runtime.InteropServices;using System.Security.Principal;
public static class PonteDeskStandardBootstrap {
 [StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)]struct SI{public int cb;public string reserved,desktop,title;public int x,y,xsize,ysize,xchars,ychars,fill,flags;public short show,reserved2;public IntPtr reserved3,input,output,error;}
 [StructLayout(LayoutKind.Sequential)]struct PI{public IntPtr process,thread;public uint pid,tid;}
 [DllImport("kernel32",SetLastError=true)]static extern IntPtr OpenProcess(uint access,bool inherit,uint pid);
 [DllImport("advapi32",SetLastError=true)]static extern bool OpenProcessToken(IntPtr process,uint access,out IntPtr token);
 [DllImport("advapi32",SetLastError=true)]static extern bool DuplicateTokenEx(IntPtr token,uint access,IntPtr attrs,int level,int type,out IntPtr duplicate);
 [DllImport("advapi32",SetLastError=true)]static extern bool GetTokenInformation(IntPtr token,int kind,out uint value,uint bytes,out uint needed);
 [DllImport("advapi32",CharSet=CharSet.Unicode,SetLastError=true)]static extern bool CreateProcessWithTokenW(IntPtr token,uint flags,string app,StringBuilder cmd,uint creation,IntPtr environment,string cwd,ref SI startup,out PI process);
 [DllImport("kernel32")]static extern bool CloseHandle(IntPtr handle);
 public static bool Elevated(uint pid){IntPtr process=OpenProcess(0x1000,false,pid),token=IntPtr.Zero;try{if(process==IntPtr.Zero||!OpenProcessToken(process,8,out token))throw new Win32Exception(Marshal.GetLastWin32Error());uint value,size;if(!GetTokenInformation(token,20,out value,4,out size))throw new Win32Exception(Marshal.GetLastWin32Error());return value!=0;}finally{if(token!=IntPtr.Zero)CloseHandle(token);if(process!=IntPtr.Zero)CloseHandle(process);}}
 public static uint Start(uint explorer,string exe,string args,string cwd){
  IntPtr process=IntPtr.Zero,token=IntPtr.Zero,primary=IntPtr.Zero;
  try{
   process=OpenProcess(0x1000,false,explorer);if(process==IntPtr.Zero)throw new Win32Exception(Marshal.GetLastWin32Error());
   if(!OpenProcessToken(process,2|8,out token))throw new Win32Exception(Marshal.GetLastWin32Error());
   if(new WindowsIdentity(token).User.Value!=WindowsIdentity.GetCurrent().User.Value)throw new InvalidOperationException("Explorer belongs to another user");
   uint elevated,size;if(!GetTokenInformation(token,20,out elevated,4,out size)||elevated!=0)throw new InvalidOperationException("Explorer token is not a standard user token");
   if(!DuplicateTokenEx(token,0x02000000,IntPtr.Zero,2,1,out primary))throw new Win32Exception(Marshal.GetLastWin32Error());
   var si=new SI();si.cb=Marshal.SizeOf(si);si.desktop="winsta0\\default";PI pi;
   if(!CreateProcessWithTokenW(primary,0,exe,new StringBuilder("\""+exe+"\" "+args),0x09000000,IntPtr.Zero,cwd,ref si,out pi))throw new Win32Exception(Marshal.GetLastWin32Error());
   CloseHandle(pi.thread);CloseHandle(pi.process);return pi.pid;
  }finally{if(primary!=IntPtr.Zero)CloseHandle(primary);if(token!=IntPtr.Zero)CloseHandle(token);if(process!=IntPtr.Zero)CloseHandle(process);}
 }
}
'@
}
$explorer=Get-Process explorer|Where-Object {$_.SessionId -eq (Get-Process -Id $PID).SessionId}|Select-Object -First 1
if(!$explorer){throw 'No interactive same-user Explorer token; refusing elevated resident fallback'}
[PonteDeskStandardBootstrap]::Start($explorer.Id,$Exe,$Arguments,(Split-Path $Exe))
