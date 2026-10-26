#include "pch.h"
#include <mfreadwrite.h>
#include <setupapi.h>
#include <cfgmgr32.h>
#include <devpkey.h>
#include <wdmguid.h>
#include <mutex>
#include <condition_variable>
#include <chrono>
#include <memory>
#include <vector>
#include <algorithm>
#include <sddl.h>
constexpr UINT MaxCaptureJpeg=2*1024*1024;
static HRESULT CaptureFailure(HRESULT hr,const std::wstring& stage) {
 wil::com_ptr_nothrow<ICreateErrorInfo> create;
 if(SUCCEEDED(CreateErrorInfo(&create))){create->SetDescription(const_cast<wchar_t*>(stage.c_str()));wil::com_ptr_nothrow<IErrorInfo> info;if(SUCCEEDED(create->QueryInterface(IID_PPV_ARGS(&info))))SetErrorInfo(0,info.get());}
 return hr;
}
struct CaptureApartment {
 bool com=false,mf=false;
 HRESULT Initialize(){auto hr=CoInitializeEx(nullptr,COINIT_MULTITHREADED);if(FAILED(hr)&&hr!=RPC_E_CHANGED_MODE)return hr;com=SUCCEEDED(hr);hr=MFStartup(MF_VERSION);mf=SUCCEEDED(hr);return hr;}
 ~CaptureApartment(){if(mf)MFShutdown();if(com)CoUninitialize();}
};
static bool ExternalUsb(const wchar_t* symbolic) {
 // MF symbolic links are opaque; resolve the interface through SetupAPI.
 auto info=SetupDiCreateDeviceInfoList(nullptr,nullptr);if(info==INVALID_HANDLE_VALUE)return false;
 auto cleanup=wil::scope_exit([&]{SetupDiDestroyDeviceInfoList(info);});
 SP_DEVICE_INTERFACE_DATA face{};face.cbSize=sizeof(face);
 if(!SetupDiOpenDeviceInterfaceW(info,symbolic,0,&face))return false;
 DWORD required=0;SetupDiGetDeviceInterfaceDetailW(info,&face,nullptr,0,&required,nullptr);
 if(!required||required>65536)return false;
 std::vector<BYTE> bytes(required);auto detail=reinterpret_cast<SP_DEVICE_INTERFACE_DETAIL_DATA_W*>(bytes.data());detail->cbSize=sizeof(*detail);
 SP_DEVINFO_DATA device{};device.cbSize=sizeof(device);
 if(!SetupDiGetDeviceInterfaceDetailW(info,&face,detail,required,nullptr,&device))return false;
 DEVPROPTYPE type=0;GUID bus{};
 if(!SetupDiGetDevicePropertyW(info,&device,&DEVPKEY_Device_BusTypeGuid,&type,reinterpret_cast<BYTE*>(&bus),sizeof(bus),nullptr,0)||bus!=GUID_BUS_TYPE_USB)return false;
 DEVINST current=device.DevInst;
 for(int depth=0;depth<8;++depth){
  wchar_t id[MAX_DEVICE_ID_LEN]={};
  if(CM_Get_Device_IDW(current,id,MAX_DEVICE_ID_LEN,0)!=CR_SUCCESS)return false;
  // Integrated USB cameras are not removable. Traverse composite parents,
  // but never accept a USB root hub or a software camera as proof.
  if(_wcsnicmp(id,L"USB\\VID_",8)==0){
   ULONG capabilities=0,size=sizeof(capabilities),kind=0;
   if(CM_Get_DevNode_Registry_PropertyW(current,CM_DRP_CAPABILITIES,&kind,&capabilities,&size,0)==CR_SUCCESS && (capabilities&CM_DEVCAP_REMOVABLE))return true;
  }
  DEVINST parent=0;if(CM_Get_Parent(&parent,current,0)!=CR_SUCCESS)break;current=parent;
 }
 return false; // unknown OEM metadata is excluded, never a builtin fallback
}
struct Device {std::wstring id,name;wil::com_ptr_nothrow<IMFActivate> activate;};
static HRESULT Devices(std::vector<Device>& devices) {
 wil::com_ptr_nothrow<IMFAttributes> attrs;RETURN_IF_FAILED(MFCreateAttributes(&attrs,1));
 RETURN_IF_FAILED(attrs->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID));
 IMFActivate** raw=nullptr;UINT count=0;RETURN_IF_FAILED(MFEnumDeviceSources(attrs.get(),&raw,&count));
 auto cleanup=wil::scope_exit([&]{for(UINT i=0;i<count;++i)raw[i]->Release();CoTaskMemFree(raw);});
 for(UINT i=0;i<count;++i){
  wil::unique_cotaskmem_string id,name;UINT idLength=0,nameLength=0;
  if(FAILED(raw[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,&id,&idLength)))continue;
  if(FAILED(raw[i]->GetAllocatedString(MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME,&name,&nameLength)))continue;
  if(!ExternalUsb(id.get()))continue;
  Device d;d.id=id.get();d.name=name.get();d.activate=raw[i];devices.emplace_back(std::move(d));
 }
 return S_OK;
}
static std::string JsonString(const std::wstring& text) {
 int size=WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,text.data(),static_cast<int>(text.size()),nullptr,0,nullptr,nullptr);
 std::string utf8(size,0);if(size)WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,text.data(),static_cast<int>(text.size()),utf8.data(),size,nullptr,nullptr);
 std::string json="\"";const char* hex="0123456789abcdef";
 for(unsigned char c:utf8){if(c=='"'||c=='\\'){json+='\\';json+=static_cast<char>(c);}else if(c<32){json+="\\u00";json+=hex[c>>4];json+=hex[c&15];}else json+=static_cast<char>(c);}return json+'"';
}
extern "C" __declspec(dllexport) HRESULT __cdecl PonteCaptureList(char* output,UINT capacity,UINT* needed) noexcept {
 if(!needed)return E_POINTER;
 try{
  CaptureApartment apartment;RETURN_IF_FAILED(apartment.Initialize());
  std::vector<Device> devices;RETURN_IF_FAILED(Devices(devices));
  std::string json="{\"devices\":[";bool first=true;
  for(const auto& device:devices){if(!first)json+=',';first=false;json+="{\"id\":"+JsonString(device.id)+",\"name\":"+JsonString(device.name)+",\"external\":true}";}json+="]}";
  *needed=static_cast<UINT>(json.size()+1);
  if(!output||!capacity)return S_OK;
  if(capacity<*needed)return HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER);
  memcpy(output,json.c_str(),*needed);return S_OK;
 }catch(const winrt::hresult_error& e){return e.code();}catch(...){return E_FAIL;}
}
struct CaptureState {
 std::mutex mutex;std::condition_variable changed;
 bool stopped=false;HRESULT error=S_OK;ULONGLONG sequence=0;
 wil::com_ptr_nothrow<IMFSample> sample;
 wil::com_ptr_nothrow<IMFSourceReader> reader;
};
struct CaptureCallback:winrt::implements<CaptureCallback,IMFSourceReaderCallback> {
 std::shared_ptr<CaptureState> state;
 explicit CaptureCallback(std::shared_ptr<CaptureState> value):state(std::move(value)){}
 STDMETHODIMP OnReadSample(HRESULT status,DWORD stream,DWORD flags,LONGLONG,IMFSample* sample) noexcept final {
  wil::com_ptr_nothrow<IMFSourceReader> reader;
  {std::lock_guard<std::mutex> lock(state->mutex);if(state->stopped)return S_OK;
   if(FAILED(status))state->error=status;
   else if(flags&MF_SOURCE_READERF_ENDOFSTREAM)state->error=MF_E_END_OF_STREAM;
   else if(sample){state->sample=sample;++state->sequence;}
   if(SUCCEEDED(state->error))reader=state->reader;
  }
  state->changed.notify_all();
  if(reader){auto hr=reader->ReadSample(stream,0,nullptr,nullptr,nullptr,nullptr);if(FAILED(hr)){std::lock_guard<std::mutex> lock(state->mutex);if(!state->stopped)state->error=hr;state->changed.notify_all();}}
  return S_OK;
 }
 STDMETHODIMP OnFlush(DWORD) noexcept final {state->changed.notify_all();return S_OK;}
 STDMETHODIMP OnEvent(DWORD,IMFMediaEvent*) noexcept final {return S_OK;}
};
struct PhysicalCapture {
 CaptureApartment apartment;
 std::shared_ptr<CaptureState> state=std::make_shared<CaptureState>();
 wil::com_ptr_nothrow<IMFMediaSource> source;
 wil::com_ptr_nothrow<IWICImagingFactory> factory;
 bool nativeJpeg=false;DWORD thread=GetCurrentThreadId();UINT width=1280,height=720;ULONGLONG last=0;
 ~PhysicalCapture(){
  wil::com_ptr_nothrow<IMFSourceReader> reader;
  {std::lock_guard<std::mutex> lock(state->mutex);state->stopped=true;reader=std::move(state->reader);state->sample.reset();}
  state->changed.notify_all();if(reader)reader->Flush(MF_SOURCE_READER_ALL_STREAMS);if(source)source->Shutdown();reader.reset();source.reset();
 }
 HRESULT Initialize(const wchar_t* id,UINT w,UINT h,UINT fps) {
  RETURN_HR_IF(E_INVALIDARG,!id||!*id||!((w==1280&&h==720)||(w==1920&&h==1080))||fps!=30);width=w;height=h;
  RETURN_IF_FAILED(apartment.Initialize());
  std::vector<Device> devices;RETURN_IF_FAILED(Devices(devices));
  auto found=std::find_if(devices.begin(),devices.end(),[&](const Device& d){return d.id==id;});
  RETURN_HR_IF(HRESULT_FROM_WIN32(ERROR_NOT_FOUND),found==devices.end());
  // This is the first actual camera open: only after explicit device selection.
  RETURN_IF_FAILED(found->activate->ActivateObject(IID_PPV_ARGS(&source)));
  wil::com_ptr_nothrow<IMFAttributes> attrs;RETURN_IF_FAILED(MFCreateAttributes(&attrs,2));
  // SourceReader may finish a queued callback after Flush/Shutdown returns.
  // Keep this small code module resident until sender-process exit; callbacks
  // own shared state, never a pointer to the already-dropped capture context.
  HMODULE pinned=nullptr;
  if(!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_PIN,
      reinterpret_cast<LPCWSTR>(&ExternalUsb),&pinned))return HRESULT_FROM_WIN32(GetLastError());
  auto callback=winrt::make_self<CaptureCallback>(state);
  RETURN_IF_FAILED(attrs->SetUnknown(MF_SOURCE_READER_ASYNC_CALLBACK,callback.get()));
  RETURN_IF_FAILED(attrs->SetUINT32(MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING,TRUE));
  wil::com_ptr_nothrow<IMFSourceReader> reader;RETURN_IF_FAILED(MFCreateSourceReaderFromMediaSource(source.get(),attrs.get(),&reader));
  RETURN_IF_FAILED(reader->SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS,FALSE));
  RETURN_IF_FAILED(reader->SetStreamSelection(MF_SOURCE_READER_FIRST_VIDEO_STREAM,TRUE));
  // Output conversion alone may leave a camera's native stream at 640x480.
  // Select the physical format before inserting the RGB converter.
  wil::com_ptr_nothrow<IMFMediaType> chosen;GUID chosenSubtype{};int best=-1;
  for(DWORD i=0;i<4096;++i){
   wil::com_ptr_nothrow<IMFMediaType> media;auto hr=reader->GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,i,&media);if(FAILED(hr))break;
   GUID subtype{};UINT mw=0,mh=0,numerator=0,denominator=0;
   media->GetGUID(MF_MT_SUBTYPE,&subtype);MFGetAttributeSize(media.get(),MF_MT_FRAME_SIZE,&mw,&mh);MFGetAttributeRatio(media.get(),MF_MT_FRAME_RATE,&numerator,&denominator);
   if(mw!=w||mh!=h||!denominator)continue;
   double rate=static_cast<double>(numerator)/denominator;if(rate<29.8||rate>30.2)continue;
   int score=subtype==MFVideoFormat_MJPG?3:subtype==MFVideoFormat_NV12?2:1;
   if(score>best){best=score;chosen=media;chosenSubtype=subtype;}
  }
  if(!chosen)return CaptureFailure(MF_E_INVALIDMEDIATYPE,L"A webcam USB nao oferece o formato nativo selecionado a 30fps");
  wil::com_ptr_nothrow<IMFSourceReaderEx> extended;
  RETURN_IF_FAILED(reader->QueryInterface(IID_PPV_ARGS(&extended)));
  DWORD streamFlags=0;auto negotiated=extended->SetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,chosen.get(),&streamFlags);
  if(FAILED(negotiated))return CaptureFailure(negotiated,L"Falha ao selecionar a resolucao fisica da webcam USB");
  nativeJpeg=chosenSubtype==MFVideoFormat_MJPG;
  if(nativeJpeg){
   negotiated=reader->SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,nullptr,chosen.get());
  }else{
   wil::com_ptr_nothrow<IMFMediaType> media;RETURN_IF_FAILED(MFCreateMediaType(&media));
   RETURN_IF_FAILED(media->SetGUID(MF_MT_MAJOR_TYPE,MFMediaType_Video));RETURN_IF_FAILED(media->SetGUID(MF_MT_SUBTYPE,MFVideoFormat_RGB32));
   negotiated=reader->SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,nullptr,media.get());
   RETURN_IF_FAILED(CoCreateInstance(CLSID_WICImagingFactory,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&factory)));
  }
  if(FAILED(negotiated))return CaptureFailure(negotiated,L"Falha ao configurar a saida da webcam USB");
  wil::com_ptr_nothrow<IMFMediaType> actual;RETURN_IF_FAILED(reader->GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,&actual));
  UINT actualWidth=0,actualHeight=0;GUID actualSubtype{};MFGetAttributeSize(actual.get(),MF_MT_FRAME_SIZE,&actualWidth,&actualHeight);actual->GetGUID(MF_MT_SUBTYPE,&actualSubtype);
  if(actualWidth!=w||actualHeight!=h||actualSubtype!=(nativeJpeg?MFVideoFormat_MJPG:MFVideoFormat_RGB32))return CaptureFailure(MF_E_INVALIDMEDIATYPE,L"Webcam USB confirmou formato diferente do selecionado; largura="+std::to_wstring(actualWidth)+L"; altura="+std::to_wstring(actualHeight));
  {std::lock_guard<std::mutex> lock(state->mutex);state->reader=reader;}
  RETURN_IF_FAILED(reader->ReadSample(MF_SOURCE_READER_FIRST_VIDEO_STREAM,0,nullptr,nullptr,nullptr,nullptr));
  return S_OK;
 }
 HRESULT Encode(IMFSample* sample,std::vector<BYTE>& jpeg) {
  wil::com_ptr_nothrow<IMFMediaBuffer> buffer;RETURN_IF_FAILED(sample->ConvertToContiguousBuffer(&buffer));
  BYTE* data=nullptr;DWORD size=0;RETURN_IF_FAILED(buffer->Lock(&data,nullptr,&size));
  auto unlock=wil::scope_exit([&]{buffer->Unlock();});
  if(nativeJpeg){
   if(size<2||size>MaxCaptureJpeg||data[0]!=0xff||data[1]!=0xd8)return CaptureFailure(E_UNEXPECTED,L"Captura JPEG: buffer inesperado; bytes="+std::to_wstring(size)+L"; SOI="+std::to_wstring(size>=2&&data[0]==0xff&&data[1]==0xd8));
   jpeg.assign(data,data+size);return S_OK;
  }
  if(size<width*height*4)return CaptureFailure(E_UNEXPECTED,L"Captura RGB32: buffer menor que o formato; bytes="+std::to_wstring(size)+L"; esperado="+std::to_wstring(width*height*4));
  wil::com_ptr_nothrow<IWICBitmap> bitmap;RETURN_IF_FAILED(factory->CreateBitmapFromMemory(width,height,GUID_WICPixelFormat32bppBGR,width*4,size,data,&bitmap));
  wil::com_ptr_nothrow<IWICFormatConverter> converter;RETURN_IF_FAILED(factory->CreateFormatConverter(&converter));
  RETURN_IF_FAILED(converter->Initialize(bitmap.get(),GUID_WICPixelFormat24bppBGR,WICBitmapDitherTypeNone,nullptr,0,WICBitmapPaletteTypeCustom));
  wil::com_ptr_nothrow<IStream> stream;RETURN_IF_FAILED(CreateStreamOnHGlobal(nullptr,TRUE,&stream));
  wil::com_ptr_nothrow<IWICBitmapEncoder> encoder;RETURN_IF_FAILED(factory->CreateEncoder(GUID_ContainerFormatJpeg,nullptr,&encoder));
  RETURN_IF_FAILED(encoder->Initialize(stream.get(),WICBitmapEncoderNoCache));
  wil::com_ptr_nothrow<IWICBitmapFrameEncode> frame;wil::com_ptr_nothrow<IPropertyBag2> properties;
  RETURN_IF_FAILED(encoder->CreateNewFrame(&frame,&properties));
  PROPBAG2 option{};option.pstrName=const_cast<LPOLESTR>(L"ImageQuality");VARIANT quality{};quality.vt=VT_R4;quality.fltVal=0.8f;
  RETURN_IF_FAILED(properties->Write(1,&option,&quality));RETURN_IF_FAILED(frame->Initialize(properties.get()));RETURN_IF_FAILED(frame->SetSize(width,height));
  WICPixelFormatGUID pixel=GUID_WICPixelFormat24bppBGR;RETURN_IF_FAILED(frame->SetPixelFormat(&pixel));
  RETURN_IF_FAILED(frame->WriteSource(converter.get(),nullptr));RETURN_IF_FAILED(frame->Commit());RETURN_IF_FAILED(encoder->Commit());
  STATSTG stat{};RETURN_IF_FAILED(stream->Stat(&stat,STATFLAG_NONAME));if(stat.cbSize.QuadPart>MaxCaptureJpeg)return CaptureFailure(E_UNEXPECTED,L"Captura RGB32: JPEG convertido excedeu 2MiB");
  LARGE_INTEGER zero{};RETURN_IF_FAILED(stream->Seek(zero,STREAM_SEEK_SET,nullptr));jpeg.resize(static_cast<size_t>(stat.cbSize.QuadPart));
  ULONG read=0;RETURN_IF_FAILED(stream->Read(jpeg.data(),static_cast<ULONG>(jpeg.size()),&read));if(read!=jpeg.size())return CaptureFailure(E_UNEXPECTED,L"Captura RGB32: leitura incompleta do JPEG convertido");return S_OK;
 }
};
extern "C" __declspec(dllexport) HRESULT __cdecl PonteCaptureStart(const wchar_t* device,UINT width,UINT height,UINT fps,void** output) noexcept {
 if(!output)return E_POINTER;*output=nullptr;
 try{auto capture=std::make_unique<PhysicalCapture>();auto hr=capture->Initialize(device,width,height,fps);if(FAILED(hr))return hr;*output=capture.release();return S_OK;}
 catch(const winrt::hresult_error& e){return e.code();}catch(...){return E_FAIL;}
}
extern "C" __declspec(dllexport) HRESULT __cdecl PonteCaptureCopy(void* opaque,BYTE* output,UINT capacity,UINT* length) noexcept {
 if(!opaque||!output||!length)return E_POINTER;*length=0;auto capture=static_cast<PhysicalCapture*>(opaque);
 if(capture->thread!=GetCurrentThreadId())return RPC_E_WRONG_THREAD;
 try{
  wil::com_ptr_nothrow<IMFSample> sample;
  {auto state=capture->state;std::unique_lock<std::mutex> lock(state->mutex);
   state->changed.wait_for(lock,std::chrono::milliseconds(250),[&]{return state->stopped||FAILED(state->error)||state->sequence!=capture->last;});
   if(FAILED(state->error))return state->error;if(state->stopped)return MF_E_SHUTDOWN;
   if(state->sequence==capture->last)return S_FALSE;capture->last=state->sequence;sample=std::move(state->sample);
  }
  if(!sample)return S_FALSE;std::vector<BYTE> jpeg;RETURN_IF_FAILED(capture->Encode(sample.get(),jpeg));
  if(jpeg.size()>capacity)return HRESULT_FROM_WIN32(ERROR_INSUFFICIENT_BUFFER);memcpy(output,jpeg.data(),jpeg.size());*length=static_cast<UINT>(jpeg.size());return S_OK;
 }catch(const winrt::hresult_error& e){return e.code();}catch(...){return E_FAIL;}
}
extern "C" __declspec(dllexport) HRESULT __cdecl PonteCaptureStop(void* opaque) noexcept {
 if(!opaque)return S_OK;auto capture=static_cast<PhysicalCapture*>(opaque);
 if(capture->thread!=GetCurrentThreadId())return RPC_E_WRONG_THREAD;
 try{delete capture;return S_OK;}catch(...){return E_FAIL;}
}
