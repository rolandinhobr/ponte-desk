#include "pch.h"
#include "FrameBus.h"
#include <new>
struct Controller {
 wil::com_ptr_nothrow<IMFVirtualCamera> camera;
 wil::com_ptr_nothrow<IWICImagingFactory> jpegFactory;
 FrameBus bus;
 DWORD thread=GetCurrentThreadId();UINT width=1920,height=1080,fps=30;
 bool com=false,mf=false,started=false;
 HANDLE producer=nullptr;bool ownsProducer=false;
 HRESULT Close(){
  HRESULT result=S_OK;
  if(started){bus.Clear();camera->Stop();result=camera->Remove();started=false;}
  camera.reset();return result;
 }
 ~Controller(){Close();jpegFactory.reset();if(mf)MFShutdown();if(com)CoUninitialize();if(ownsProducer)ReleaseMutex(producer);if(producer)CloseHandle(producer);}
};
extern "C" __declspec(dllexport) HRESULT __cdecl PonteWebcamStart(UINT width,UINT height,UINT fps,void** result) noexcept {
 if(!result)return E_POINTER;*result=nullptr;
 if(!((width==1280&&height==720)||(width==1920&&height==1080))||fps!=30)return E_INVALIDARG;
 try {
  auto context=std::unique_ptr<Controller>(new(std::nothrow)Controller());if(!context)return E_OUTOFMEMORY;
  context->width=width;context->height=height;context->fps=fps;
  context->producer=CreateMutexW(nullptr,FALSE,L"Local\\PonteDeskWebcamProducer-93e78377");
  if(!context->producer)return HRESULT_FROM_WIN32(GetLastError());
  auto lock=WaitForSingleObject(context->producer,0);
  if(lock!=WAIT_OBJECT_0 && lock!=WAIT_ABANDONED)return HRESULT_FROM_WIN32(ERROR_BUSY);
  context->ownsProducer=true;
  auto hr=CoInitializeEx(nullptr,COINIT_MULTITHREADED);
  if(FAILED(hr) && hr!=RPC_E_CHANGED_MODE)return hr;context->com=SUCCEEDED(hr);
  hr=MFStartup(MF_VERSION,MFSTARTUP_FULL);if(FAILED(hr))return hr;context->mf=true;
  hr=CoCreateInstance(CLSID_WICImagingFactory,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&context->jpegFactory));if(FAILED(hr))return hr;
  hr=MFCreateVirtualCamera(MFVirtualCameraType_SoftwareCameraSource,MFVirtualCameraLifetime_Session,MFVirtualCameraAccess_CurrentUser,L"Ponte Desk Camera",L"{93E78377-4F8D-45DB-A4B7-618B2271189A}",nullptr,0,&context->camera);
  if(FAILED(hr))return hr;
  hr=context->camera->Start(nullptr);if(FAILED(hr))return hr;
  context->started=true;context->bus.Clear();
  *result=context.release();return S_OK;
 }catch(const winrt::hresult_error& error){return error.code();}catch(...){return E_FAIL;}
}
static HRESULT CheckJpeg(Controller* context,const BYTE* jpeg,UINT bytes){
 if(!jpeg||bytes<4||bytes>PonteMaxJpeg||jpeg[0]!=0xff||jpeg[1]!=0xd8)return E_INVALIDARG;
 wil::com_ptr_nothrow<IWICStream> stream;RETURN_IF_FAILED(context->jpegFactory->CreateStream(&stream));
 RETURN_IF_FAILED(stream->InitializeFromMemory(const_cast<BYTE*>(jpeg),bytes));
 wil::com_ptr_nothrow<IWICBitmapDecoder> decoder;RETURN_IF_FAILED(context->jpegFactory->CreateDecoderFromStream(stream.get(),nullptr,WICDecodeMetadataCacheOnDemand,&decoder));
 wil::com_ptr_nothrow<IWICBitmapFrameDecode> frame;RETURN_IF_FAILED(decoder->GetFrame(0,&frame));
 UINT width=0,height=0;RETURN_IF_FAILED(frame->GetSize(&width,&height));
 if((width==1280&&height==720)||(width==1920&&height==1080))return S_OK;
 wil::com_ptr_nothrow<ICreateErrorInfo> error;
 if(SUCCEEDED(CreateErrorInfo(&error))){
  error->SetSource(const_cast<LPOLESTR>(L"Ponte Desk Camera"));
  error->SetDescription(const_cast<LPOLESTR>(L"JPEG da webcam deve ter 1280x720 ou 1920x1080; a resolucao de saida e negociada pelo aplicativo."));
  wil::com_ptr_nothrow<IErrorInfo> info;if(SUCCEEDED(error->QueryInterface(&info)))SetErrorInfo(0,info.get());
 }
 WinTrace(2,0,L"Ponte Desk Camera: JPEG dimensions unsupported; expected 1280x720 or 1920x1080.");
 return MF_E_INVALIDMEDIATYPE;
}
extern "C" __declspec(dllexport) HRESULT __cdecl PonteWebcamPush(void* opaque,const BYTE* jpeg,UINT bytes) noexcept {
 if(!opaque)return E_POINTER;
 auto context=static_cast<Controller*>(opaque);
 if(context->thread!=GetCurrentThreadId())return RPC_E_WRONG_THREAD;
 try{auto hr=CheckJpeg(context,jpeg,bytes);if(FAILED(hr))return hr;return context->bus.Push(jpeg,bytes,context->width,context->height,context->fps);}catch(...){return E_FAIL;}
}
extern "C" __declspec(dllexport) HRESULT __cdecl PonteWebcamStop(void* opaque) noexcept {
 if(!opaque)return S_OK;
 auto context=static_cast<Controller*>(opaque);
 if(context->thread!=GetCurrentThreadId())return RPC_E_WRONG_THREAD;
 try{auto hr=context->Close();delete context;return hr;}catch(...){return E_FAIL;}
}
