#include "pch.h"
#include "Tools.h"
#include "FrameGenerator.h"
HRESULT FrameGenerator::EnsureRenderTarget(UINT width,UINT height) {
 bool changed=width!=_width||height!=_height||_pixels.empty();
 _width=width;_height=height;
 if(!_factory) RETURN_IF_FAILED(CoCreateInstance(CLSID_WICImagingFactory,nullptr,CLSCTX_INPROC_SERVER,IID_PPV_ARGS(&_factory)));
 if(changed){_pixels.assign(static_cast<size_t>(width)*height*4,0);_sequence=0;_updated=0;}
 // Session-zero FrameServer creates anonymous shared memory; no image files.
 auto hr=_bus.Open(true);
 // Windows may probe a custom source in the unelevated client before its
 // FrameServer host starts it. That probe cannot create a Global section.
 // Keep metadata activation valid; FrameServer/session zero creates it later,
 // and RequestSample retries the mapping without a privileged producer.
 WINTRACE(L"FrameBus initialize process:%u hr:0x%08X",GetCurrentProcessId(),hr);
 return S_OK;
}
HRESULT FrameGenerator::Decode() {
 wil::com_ptr_nothrow<IWICStream> stream;
 RETURN_IF_FAILED(_factory->CreateStream(&stream));
 RETURN_IF_FAILED(stream->InitializeFromMemory(_jpeg.data(),static_cast<DWORD>(_jpeg.size())));
 wil::com_ptr_nothrow<IWICBitmapDecoder> decoder;
 RETURN_IF_FAILED(_factory->CreateDecoderFromStream(stream.get(),nullptr,WICDecodeMetadataCacheOnLoad,&decoder));
 wil::com_ptr_nothrow<IWICBitmapFrameDecode> frame;
 RETURN_IF_FAILED(decoder->GetFrame(0,&frame));
 UINT width,height; RETURN_IF_FAILED(frame->GetSize(&width,&height));
 RETURN_HR_IF(MF_E_INVALIDMEDIATYPE,!((width==1280&&height==720)||(width==1920&&height==1080)));
 wil::com_ptr_nothrow<IWICFormatConverter> converter;
 RETURN_IF_FAILED(_factory->CreateFormatConverter(&converter));
 RETURN_IF_FAILED(converter->Initialize(frame.get(),GUID_WICPixelFormat32bppBGRA,WICBitmapDitherTypeNone,nullptr,0,WICBitmapPaletteTypeCustom));
 if(width==_width&&height==_height){
  RETURN_IF_FAILED(converter->CopyPixels(nullptr,_width*4,static_cast<UINT>(_pixels.size()),_pixels.data()));
 }else{
  wil::com_ptr_nothrow<IWICBitmapScaler> scaler;
  RETURN_IF_FAILED(_factory->CreateBitmapScaler(&scaler));
  RETURN_IF_FAILED(scaler->Initialize(converter.get(),_width,_height,WICBitmapInterpolationModeFant));
  RETURN_IF_FAILED(scaler->CopyPixels(nullptr,_width*4,static_cast<UINT>(_pixels.size()),_pixels.data()));
 }
 return S_OK;
}
HRESULT FrameGenerator::Generate(IMFSample* sample,REFGUID format,IMFSample** output) {
 RETURN_HR_IF_NULL(E_POINTER,sample);RETURN_HR_IF_NULL(E_POINTER,output);*output=nullptr;
 auto previous=_sequence;
 bool live=_bus.Read(_jpeg,_sequence,_updated);
 if(!live && (!_updated || GetTickCount64()-_updated>2000)) std::fill(_pixels.begin(),_pixels.end(),0);
 if(live && previous!=_sequence && FAILED(Decode())) {std::fill(_pixels.begin(),_pixels.end(),0);_updated=0;}
 wil::com_ptr_nothrow<IMFMediaBuffer> buffer;
 RETURN_IF_FAILED(sample->GetBufferByIndex(0,&buffer));
 wil::com_ptr_nothrow<IMF2DBuffer2> buffer2d;
 RETURN_IF_FAILED(buffer->QueryInterface(IID_PPV_ARGS(&buffer2d)));
 BYTE *scanline,*start;LONG pitch;DWORD length;
 RETURN_IF_FAILED(buffer2d->Lock2DSize(MF2DBuffer_LockFlags_Write,&scanline,&pitch,&start,&length));
 auto unlock=wil::scope_exit([&]{buffer2d->Unlock2D();});
 RETURN_HR_IF(E_UNEXPECTED,pitch<static_cast<LONG>(_width));
 if(format==MFVideoFormat_NV12) {
  RETURN_HR_IF(E_UNEXPECTED,length<static_cast<UINT>(pitch)*_height*3/2);
  RETURN_IF_FAILED(RGB32ToNV12(_pixels.data(),static_cast<ULONG>(_pixels.size()),_width*4,_width,_height,scanline,length,pitch));
 } else {
  RETURN_HR_IF(E_UNEXPECTED,pitch<static_cast<LONG>(_width*4) || length<static_cast<UINT>(pitch)*_height);
  for(UINT y=0;y<_height;++y) memcpy(scanline+y*pitch,_pixels.data()+static_cast<size_t>(y)*_width*4,_width*4);
 }
 sample->AddRef();*output=sample;return S_OK;
}
