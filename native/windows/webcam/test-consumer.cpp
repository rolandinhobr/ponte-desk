#define NOMINMAX
#include <windows.h>
#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <mferror.h>
#include <cwctype>
#include <wrl.h>
#include <chrono>
#include <thread>
#include <mutex>
#include <condition_variable>
#include <memory>
#include <unordered_set>
#include <string>
#include <algorithm>
#include <cstdio>
using Microsoft::WRL::ComPtr;
struct State {std::mutex mutex;std::condition_variable ready;bool arrived=false;HRESULT error=S_OK;DWORD flags=0;ComPtr<IMFSample> sample;};
class Callback final:public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IMFSourceReaderCallback>{
 std::shared_ptr<State> state;
public:explicit Callback(std::shared_ptr<State> value):state(std::move(value)){}
 STDMETHODIMP OnReadSample(HRESULT error,DWORD,DWORD flags,LONGLONG,IMFSample* sample) override{
  {std::lock_guard<std::mutex> lock(state->mutex);state->error=error;state->flags=flags;state->sample=sample;state->arrived=true;}state->ready.notify_one();return S_OK;
 }
 STDMETHODIMP OnEvent(DWORD,IMFMediaEvent*) override{return S_OK;}
 STDMETHODIMP OnFlush(DWORD) override{return S_OK;}
};
static UINT ponteNamesSeen=0,exactNamesSeen=0,usbIdentitiesRejected=0;
static std::string candidateName;
static bool ExactPonteName(const wchar_t* name){
 // MF appends its own suffix; every accepted name is an exact branded name.
 return wcscmp(name,L"Ponte Desk Camera (C\u00e2mera Virtual do\u00a0Windows)")==0||wcscmp(name,L"Ponte Desk Camera")==0||wcscmp(name,L"Ponte Desk Camera Windows Virtual Camera")==0||wcscmp(name,L"Ponte Desk Camera (Windows Virtual Camera)")==0||wcscmp(name,L"PonteDeskWebcam")==0;
}
static HRESULT FindPonte(ComPtr<IMFActivate>& selected){
 ComPtr<IMFAttributes> attrs;auto hr=MFCreateAttributes(&attrs,1);if(FAILED(hr))return hr;
 hr=attrs->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);if(FAILED(hr))return hr;
 IMFActivate** devices=nullptr;UINT count=0;hr=MFEnumDeviceSources(attrs.Get(),&devices,&count);if(FAILED(hr))return hr;
 for(UINT i=0;i<count;++i){
  wchar_t* name=nullptr;UINT length=0;
  if(SUCCEEDED(devices[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,&name,&length))){
   if(wcsstr(name,L"Ponte")){
    ++ponteNamesSeen;int bytes=WideCharToMultiByte(CP_UTF8,0,name,-1,nullptr,0,nullptr,nullptr);
    if(bytes>1){candidateName.resize(bytes);WideCharToMultiByte(CP_UTF8,0,name,-1,candidateName.data(),bytes,nullptr,nullptr);candidateName.resize(bytes-1);}
   }
   if(ExactPonteName(name)){
    ++exactNamesSeen;
    wchar_t* link=nullptr;UINT linkLength=0;bool usb=false;
    if(SUCCEEDED(devices[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,&link,&linkLength))){std::wstring lower(link);std::transform(lower.begin(),lower.end(),lower.begin(),[](wchar_t c){return static_cast<wchar_t>(towlower(c));});usb=lower.find(L"usb#")!=std::wstring::npos;CoTaskMemFree(link);}
    if(!usb)selected=devices[i];else ++usbIdentitiesRejected;
   }
   CoTaskMemFree(name);
  }
  devices[i]->Release();
 }
 CoTaskMemFree(devices);return selected?S_OK:S_FALSE;
}
static UINT targetWidth=1920,targetHeight=1080;
struct Metrics {UINT frames=0,nonempty=0,spatial=0,changes=0,unique=0;double maximumVariance=0;DWORD minimumBytes=MAXDWORD;bool elevated=true,found=false,format=false;HRESULT error=S_OK;};
static bool Elevated(){HANDLE token=nullptr;TOKEN_ELEVATION value{};DWORD size=0;bool elevated=true;if(OpenProcessToken(GetCurrentProcess(),TOKEN_QUERY,&token)){if(GetTokenInformation(token,TokenElevation,&value,sizeof(value),&size))elevated=value.TokenIsElevated!=0;CloseHandle(token);}return elevated;}
static HRESULT Consume(Metrics& m){
 ComPtr<IMFActivate> selected;auto until=std::chrono::steady_clock::now()+std::chrono::seconds(15);
 do{auto hr=FindPonte(selected);if(FAILED(hr))return hr;if(selected)break;std::this_thread::sleep_for(std::chrono::milliseconds(200));}while(std::chrono::steady_clock::now()<until);
 if(!selected)return HRESULT_FROM_WIN32(ERROR_NOT_FOUND);m.found=true;
 ComPtr<IMFMediaSource> source;auto hr=selected->ActivateObject(IID_PPV_ARGS(&source));if(FAILED(hr))return hr;
 auto state=std::make_shared<State>();auto callback=Microsoft::WRL::Make<Callback>(state);
 ComPtr<IMFAttributes> attrs;hr=MFCreateAttributes(&attrs,1);if(FAILED(hr))return hr;hr=attrs->SetUnknown(MF_SOURCE_READER_ASYNC_CALLBACK,callback.Get());if(FAILED(hr))return hr;
 ComPtr<IMFSourceReader> reader;hr=MFCreateSourceReaderFromMediaSource(source.Get(),attrs.Get(),&reader);if(FAILED(hr))return hr;
 ComPtr<IMFMediaType> type;hr=MFCreateMediaType(&type);if(FAILED(hr))return hr;
 type->SetGUID(MF_MT_MAJOR_TYPE,MFMediaType_Video);type->SetGUID(MF_MT_SUBTYPE,MFVideoFormat_NV12);MFSetAttributeSize(type.Get(),MF_MT_FRAME_SIZE,targetWidth,targetHeight);MFSetAttributeRatio(type.Get(),MF_MT_FRAME_RATE,30,1);
 hr=reader->SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,nullptr,type.Get());if(FAILED(hr))return hr;
 ComPtr<IMFMediaType> actual;hr=reader->GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,&actual);if(FAILED(hr))return hr;
 UINT width=0,height=0;GUID subtype{};actual->GetGUID(MF_MT_SUBTYPE,&subtype);MFGetAttributeSize(actual.Get(),MF_MT_FRAME_SIZE,&width,&height);
 if(width!=targetWidth||height!=targetHeight||subtype!=MFVideoFormat_NV12)return MF_E_INVALIDMEDIATYPE;m.format=true;
 std::unordered_set<unsigned long long> hashes;unsigned long long previous=0;auto deadline=std::chrono::steady_clock::now()+std::chrono::seconds(10);
 for(UINT attempt=0;attempt<180&&m.frames<60&&std::chrono::steady_clock::now()<deadline;++attempt){
  {std::lock_guard<std::mutex> lock(state->mutex);state->arrived=false;state->sample.Reset();}
  hr=reader->ReadSample(MF_SOURCE_READER_FIRST_VIDEO_STREAM,0,nullptr,nullptr,nullptr,nullptr);if(FAILED(hr))break;
  ComPtr<IMFSample> sample;
  {std::unique_lock<std::mutex> lock(state->mutex);if(!state->ready.wait_for(lock,std::chrono::seconds(3),[&]{return state->arrived;})){hr=HRESULT_FROM_WIN32(ERROR_TIMEOUT);break;}hr=state->error;sample=std::move(state->sample);if(state->flags&MF_SOURCE_READERF_ENDOFSTREAM)hr=MF_E_END_OF_STREAM;}
  if(FAILED(hr))break;if(!sample)continue;
  ComPtr<IMFMediaBuffer> buffer;hr=sample->ConvertToContiguousBuffer(&buffer);if(FAILED(hr))break;
  BYTE* pixels=nullptr;DWORD bytes=0;hr=buffer->Lock(&pixels,nullptr,&bytes);if(FAILED(hr))break;
  ++m.frames;m.minimumBytes=std::min(m.minimumBytes,bytes);
  if(bytes>=targetWidth*targetHeight*3/2){
   ++m.nonempty;unsigned long long hash=1469598103934665603ull;double sum=0,squared=0;
   for(UINT i=0;i<targetWidth*targetHeight;++i){double y=pixels[i];sum+=y;squared+=y*y;}
   double variance=squared/(static_cast<double>(targetWidth)*targetHeight)-((sum/(static_cast<double>(targetWidth)*targetHeight))*(sum/(static_cast<double>(targetWidth)*targetHeight)));
   m.maximumVariance=std::max(m.maximumVariance,variance);if(variance>0.1)++m.spatial;
   for(DWORD i=0;i<bytes;++i){hash^=pixels[i];hash*=1099511628211ull;}
   hashes.insert(hash);if(m.frames>1&&hash!=previous)++m.changes;previous=hash;
  }
  buffer->Unlock();std::this_thread::sleep_for(std::chrono::milliseconds(33));
 }
 m.unique=static_cast<UINT>(hashes.size());reader->Flush(MF_SOURCE_READER_ALL_STREAMS);source->Shutdown();reader.Reset();source.Reset();return hr;
}
int wmain(int argc,wchar_t** argv){
 if(argc<2)return 2;if(argc>2&&wcscmp(argv[2],L"720")==0){targetWidth=1280;targetHeight=720;}Metrics m;m.elevated=Elevated();HRESULT init=CoInitializeEx(nullptr,COINIT_MULTITHREADED);bool com=SUCCEEDED(init),mf=false;
 if(SUCCEEDED(init)){init=MFStartup(MF_VERSION);mf=SUCCEEDED(init);}if(SUCCEEDED(init))m.error=Consume(m);else m.error=init;
 if(mf)MFShutdown();if(com)CoUninitialize();
 bool passed=SUCCEEDED(m.error)&&!m.elevated&&m.found&&m.format&&m.frames==60&&m.nonempty==60&&m.spatial>0&&m.unique>1&&m.changes>0;
 char json[4096];int size=snprintf(json,sizeof(json),"{\"passed\":%s,\"consumer\":\"MediaFoundation\",\"consumer_elevated\":%s,\"found_exact_ponte_virtual_camera\":%s,\"format_nv12_matches_request\":%s,\"width\":%u,\"height\":%u,\"frames\":%u,\"nonempty_frames\":%u,\"spatial_variance_frames\":%u,\"maximum_spatial_variance\":%.3f,\"distinct_frame_checksums\":%u,\"frame_checksum_changes\":%u,\"minimum_sample_bytes\":%u,\"hresult\":\"0x%08X\",\"physical_windows_camera_opened\":false,\"image_files_created\":false,\"raw_checksums_or_pixels_recorded\":false,\"ponte_names_seen\":%u,\"exact_names_seen\":%u,\"usb_identities_rejected\":%u,\"candidate_name\":\"%s\"}\n",passed?"true":"false",m.elevated?"true":"false",m.found?"true":"false",m.format?"true":"false",targetWidth,targetHeight,m.frames,m.nonempty,m.spatial,m.maximumVariance,m.unique,m.changes,m.minimumBytes==MAXDWORD?0:m.minimumBytes,static_cast<UINT>(m.error),ponteNamesSeen,exactNamesSeen,usbIdentitiesRejected,candidateName.c_str());
 auto file=CreateFileW(argv[1],GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_ALWAYS,FILE_ATTRIBUTE_NORMAL,nullptr);if(file==INVALID_HANDLE_VALUE)return 3;DWORD written=0;WriteFile(file,json,static_cast<DWORD>(size),&written,nullptr);CloseHandle(file);return passed?0:4;
}
