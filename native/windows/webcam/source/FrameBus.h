#pragma once
#include <sddl.h>
#include <vector>
constexpr ULONG PonteMaxJpeg=2*1024*1024;
constexpr ULONG PonteFrameMagic=0x50445743;
constexpr wchar_t PonteMapName[]=L"Global\\PonteDeskWebcam-93e78377-4f8d-45db-a4b7-618b2271189a";
constexpr wchar_t PonteMutexName[]=L"Global\\PonteDeskWebcamMutex-93e78377-4f8d-45db-a4b7-618b2271189a";
constexpr wchar_t PonteLocalMapName[]=L"Local\\PonteDeskWebcam-93e78377-4f8d-45db-a4b7-618b2271189a";
constexpr wchar_t PonteLocalMutexName[]=L"Local\\PonteDeskWebcamMutex-93e78377-4f8d-45db-a4b7-618b2271189a";
struct SharedFrame {
 ULONG magic,active,width,height,fps,jpegSize;
 ULONGLONG sequence,updated;
 BYTE jpeg[PonteMaxJpeg];
};
class FrameSlot {
 const wchar_t* _mapName;const wchar_t* _mutexName;
 HANDLE _mapping=nullptr,_mutex=nullptr;
 SharedFrame* _frame=nullptr;
public:
 FrameSlot(const wchar_t* mapName,const wchar_t* mutexName):_mapName(mapName),_mutexName(mutexName){}
 FrameSlot(const FrameSlot&)=delete;FrameSlot& operator=(const FrameSlot&)=delete;
 ~FrameSlot(){if(_frame)UnmapViewOfFile(_frame);if(_mapping)CloseHandle(_mapping);if(_mutex)CloseHandle(_mutex);}
 HRESULT Open(bool create,bool* sectionCreationDenied=nullptr){
  if(sectionCreationDenied)*sectionCreationDenied=false;
  if(_frame)return S_OK;
  HANDLE mapping=nullptr,mutex=nullptr;
  if(create){
   wchar_t sid[256]={};DWORD bytes=sizeof(sid);
   auto status=RegGetValueW(HKEY_LOCAL_MACHINE,L"Software\\Classes\\CLSID\\{93E78377-4F8D-45DB-A4B7-618B2271189A}\\InprocServer32",L"OwnerSid",RRF_RT_REG_SZ,nullptr,sid,&bytes);
   if(status!=ERROR_SUCCESS)return HRESULT_FROM_WIN32(status);
   std::wstring sddl=L"D:P(A;;GA;;;SY)(A;;GA;;;LS)(A;;GA;;;"+std::wstring(sid)+L")";
   PSECURITY_DESCRIPTOR descriptor=nullptr;
   if(!ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(),SDDL_REVISION_1,&descriptor,nullptr))return HRESULT_FROM_WIN32(GetLastError());
   auto freeDescriptor=wil::scope_exit([&]{LocalFree(descriptor);});
   SECURITY_ATTRIBUTES security{sizeof(security),descriptor,FALSE};
   // Publish the mutex before the section so a producer never sees half setup.
   mutex=CreateMutexW(&security,FALSE,_mutexName);
   if(!mutex)return HRESULT_FROM_WIN32(GetLastError());
   mapping=CreateFileMappingW(INVALID_HANDLE_VALUE,&security,PAGE_READWRITE,0,sizeof(SharedFrame),_mapName);
   if(!mapping){auto error=GetLastError();CloseHandle(mutex);
    if(sectionCreationDenied&&(error==ERROR_ACCESS_DENIED||error==ERROR_PRIVILEGE_NOT_HELD))*sectionCreationDenied=true;
    return HRESULT_FROM_WIN32(error);
   }
  }else{
   mapping=OpenFileMappingW(FILE_MAP_READ|FILE_MAP_WRITE,FALSE,_mapName);
   if(!mapping){auto error=GetLastError();return error==ERROR_FILE_NOT_FOUND?S_FALSE:HRESULT_FROM_WIN32(error);}
   mutex=OpenMutexW(SYNCHRONIZE|MUTEX_MODIFY_STATE,FALSE,_mutexName);
   if(!mutex){auto error=GetLastError();CloseHandle(mapping);return error==ERROR_FILE_NOT_FOUND?S_FALSE:HRESULT_FROM_WIN32(error);}
  }
  auto frame=static_cast<SharedFrame*>(MapViewOfFile(mapping,FILE_MAP_READ|FILE_MAP_WRITE,0,0,sizeof(SharedFrame)));
  if(!frame){auto error=GetLastError();CloseHandle(mutex);CloseHandle(mapping);return HRESULT_FROM_WIN32(error);}
  _mapping=mapping;_mutex=mutex;_frame=frame;return S_OK;
 }
 HRESULT Push(const BYTE* jpeg,ULONG bytes,UINT width,UINT height,UINT fps){
  auto hr=Open(false);if(hr!=S_OK)return hr;
  auto wait=WaitForSingleObject(_mutex,0);if(wait!=WAIT_OBJECT_0&&wait!=WAIT_ABANDONED)return S_FALSE;
  auto unlock=wil::scope_exit([&]{ReleaseMutex(_mutex);});
  _frame->magic=PonteFrameMagic;_frame->width=width;_frame->height=height;_frame->fps=fps;
  memcpy(_frame->jpeg,jpeg,bytes);_frame->jpegSize=bytes;_frame->updated=GetTickCount64();++_frame->sequence;_frame->active=1;return S_OK;
 }
 bool Read(std::vector<BYTE>& jpeg,ULONGLONG& sequence,ULONGLONG& updated){
  auto wait=WaitForSingleObject(_mutex,0);if(wait!=WAIT_OBJECT_0&&wait!=WAIT_ABANDONED)return false;
  auto unlock=wil::scope_exit([&]{ReleaseMutex(_mutex);});
  if(_frame->magic!=PonteFrameMagic||!_frame->active||!_frame->jpegSize||_frame->jpegSize>PonteMaxJpeg||GetTickCount64()-_frame->updated>2000){updated=0;return false;}
  updated=_frame->updated;if(sequence==_frame->sequence)return true;
  jpeg.assign(_frame->jpeg,_frame->jpeg+_frame->jpegSize);sequence=_frame->sequence;return true;
 }
 void Clear(){
  if(Open(false)!=S_OK)return;auto wait=WaitForSingleObject(_mutex,100);
  if(wait==WAIT_OBJECT_0||wait==WAIT_ABANDONED){SecureZeroMemory(_frame,sizeof(SharedFrame));ReleaseMutex(_mutex);}
 }
};
class FrameBus {
 FrameSlot _global{PonteMapName,PonteMutexName};
 FrameSlot _local{PonteLocalMapName,PonteLocalMutexName};
 FrameSlot* _reader=nullptr;
public:
 HRESULT Open(bool create){
  if(create){
   if(_reader)return S_OK;
   bool creationDenied=false;auto hr=_global.Open(true,&creationDenied);
   if(hr==S_OK){_reader=&_global;return S_OK;}
   // MF can instantiate the source directly in an ordinary client, not only
   // in session-zero FrameServer. Fall back only for this creation privilege.
   if(!creationDenied)return hr;
   hr=_local.Open(true);if(hr==S_OK)_reader=&_local;return hr;
  }
  auto global=_global.Open(false),local=_local.Open(false);
  if(global==S_OK||local==S_OK)return S_OK;
  return FAILED(global)?global:local;
 }
 HRESULT Push(const BYTE* jpeg,ULONG bytes,UINT width,UINT height,UINT fps){
  RETURN_HR_IF(E_INVALIDARG,!jpeg||!bytes||bytes>PonteMaxJpeg);
  // Reader namespace stays fixed. Feed both bounded slots when both exist,
  // supporting session-zero FrameServer and same-session ordinary clients.
  auto global=_global.Push(jpeg,bytes,width,height,fps),local=_local.Push(jpeg,bytes,width,height,fps);
  if(global==S_OK||local==S_OK)return S_OK;
  if(FAILED(global))return global;return local;
 }
 bool Read(std::vector<BYTE>& jpeg,ULONGLONG& sequence,ULONGLONG& updated){
  if(Open(true)!=S_OK)return false;return _reader->Read(jpeg,sequence,updated);
 }
 void Clear(){_global.Clear();_local.Clear();}
};
