#include "source/pch.h"
#include "source/FrameBus.h"
#include <cstdio>
int wmain(int argc,wchar_t** argv){
 if(argc!=2)return 2;HANDLE token=nullptr;TOKEN_ELEVATION elevation{};DWORD bytes=0;
 if(OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&token)){GetTokenInformation(token,TokenElevation,&elevation,sizeof(elevation),&bytes);CloseHandle(token);}
 HANDLE existing=OpenFileMappingW(FILE_MAP_READ|FILE_MAP_WRITE,FALSE,PonteMapName);DWORD beforeError=existing?ERROR_SUCCESS:GetLastError();if(existing)CloseHandle(existing);
 wchar_t sid[256]={};DWORD sidBytes=sizeof(sid);
 DWORD registryError=RegGetValueW(HKEY_LOCAL_MACHINE,L"Software\\Classes\\CLSID\\{93E78377-4F8D-45DB-A4B7-618B2271189A}\\InprocServer32",L"OwnerSid",RRF_RT_REG_SZ,nullptr,sid,&sidBytes);
 std::wstring sddl=L"D:P(A;;GA;;;SY)(A;;GA;;;LS)(A;;GA;;;"+std::wstring(sid)+L")";
 PSECURITY_DESCRIPTOR descriptor=nullptr;BOOL sddlOk=ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(),SDDL_REVISION_1,&descriptor,nullptr);DWORD sddlError=sddlOk?0:GetLastError();
 SECURITY_ATTRIBUTES security{sizeof(security),descriptor,FALSE};
 HANDLE created=CreateFileMappingW(INVALID_HANDLE_VALUE,&security,PAGE_READWRITE,0,sizeof(SharedFrame),PonteMapName);DWORD createError=created?0:GetLastError();
 HANDLE mutex=CreateMutexW(&security,FALSE,PonteMutexName);DWORD mutexError=mutex?0:GetLastError();if(mutex)CloseHandle(mutex);if(created)CloseHandle(created);if(descriptor)LocalFree(descriptor);
 FrameBus exact;HRESULT hr=exact.Open(true);DWORD afterError=GetLastError();
 wchar_t localName[128];swprintf_s(localName,L"Local\\PonteDeskWebcamDiagnostic-%u",GetCurrentProcessId());
 HANDLE local=CreateFileMappingW(INVALID_HANDLE_VALUE,nullptr,PAGE_READWRITE,0,4096,localName);DWORD localError=local?ERROR_SUCCESS:GetLastError();if(local)CloseHandle(local);
 char json[1024];int length=snprintf(json,sizeof(json),"{\"elevated\":%s,\"global_exists_before\":%s,\"global_open_error\":%u,\"exact_FrameBus_Open_create_hresult\":\"0x%08X\",\"last_error_after_call\":%u,\"local_create_error\":%u,\"registry_read_error\":%u,\"sddl_error\":%u,\"CreateFileMapping_Global_error\":%u,\"CreateMutex_Global_error\":%u,\"image_data_accessed\":false}\n",elevation.TokenIsElevated?"true":"false",beforeError==0?"true":"false",beforeError,static_cast<UINT>(hr),afterError,localError,registryError,sddlError,createError,mutexError);
 HANDLE report=CreateFileW(argv[1],GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_ALWAYS,FILE_ATTRIBUTE_NORMAL,nullptr);if(report==INVALID_HANDLE_VALUE)return 3;DWORD written=0;WriteFile(report,json,length,&written,nullptr);CloseHandle(report);return 0;
}
