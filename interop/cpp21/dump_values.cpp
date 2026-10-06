// qi-dump-values (libqi 2.1): serialize a catalogue of values and messages
// with the binary codec of NAOqi 2.1's libqi, one JSON object per line, in the
// format of interop/cpp/dump_values.cpp (same names for the same values, so
// that the two catalogues can be compared line by line).
//
// Specific to 2.1: the ServiceInfo has no objectUid field, the default
// capabilities are ClientServerSocket/MetaObjectCache/MessageFlags, there is
// no authentication (the server answers the authentication call with an
// error, dumped as msg_error_cant_find_service), no cancel messages, no
// optionals and no object UIDs. Object references are dumped in both of their
// forms: without the MetaObjectCache capability (metaObject, serviceId,
// objectId) and with it (transmit flag, [metaObject], cacheId, serviceId,
// objectId).

#include <cstdint>
#include <cstring>
#include <iostream>
#include <map>
#include <string>
#include <utility>
#include <vector>

#include <boost/bind.hpp>

#include <qi/application.hpp>
#include <qi/buffer.hpp>
#include <qi/os.hpp>
#include <qimessaging/serviceinfo.hpp>
#include <qimessaging/session.hpp>
#include <qitype/anyobject.hpp>
#include <qitype/anyvalue.hpp>
#include <qitype/binarycodec.hpp>
#include <qitype/dynamicobject.hpp>
#include <qitype/dynamicobjectbuilder.hpp>
#include <qitype/jsoncodec.hpp>
#include <qitype/metaobject.hpp>
#include <qitype/signature.hpp>


#include "interop_types.hpp"
#include "json.hpp"
#include "stderr_log.hpp"

namespace
{
  /// Flatten a qi::Buffer as the transport sends it: sub-buffers inlined after
  /// their uint32 size prefix.
  std::vector<std::uint8_t> flatten(const qi::Buffer& buf)
  {
    std::vector<std::uint8_t> out;
    const std::uint8_t* data = static_cast<const std::uint8_t*>(buf.data());
    size_t begin = 0;
    const std::vector<std::pair<size_t, qi::Buffer> >& subs = buf.subBuffers();
    for (size_t i = 0; i < subs.size(); ++i)
    {
      const size_t end = subs[i].first + sizeof(std::uint32_t);
      if (data && end > begin)
        out.insert(out.end(), data + begin, data + end);
      begin = end;
      const std::vector<std::uint8_t> subBytes = flatten(subs[i].second);
      out.insert(out.end(), subBytes.begin(), subBytes.end());
    }
    if (data && buf.size() > begin)
      out.insert(out.end(), data + begin, data + buf.size());
    return out;
  }

  std::string jsonOf(const qi::AutoAnyReference& ref)
  {
    try
    {
      return qi::encodeJSON(ref);
    }
    catch (const std::exception&)
    {
      return "";
    }
  }

  void emitValue(const std::string& name, const std::string& signature, const qi::Buffer& buf, const std::string& json)
  {
    std::vector<std::pair<std::string, std::string> > fields;
    fields.push_back(std::make_pair("name", ijson::str(name)));
    fields.push_back(std::make_pair("signature", ijson::str(signature)));
    fields.push_back(std::make_pair("hex", ijson::str(ijson::hex(flatten(buf)))));
    if (!json.empty())
      fields.push_back(std::make_pair("qi_json", ijson::str(json)));
    std::cout << ijson::object(fields) << std::endl;
  }

  void emitError(const std::string& name, const std::string& error)
  {
    std::cout << ijson::object({{"name", ijson::str(name)}, {"error", ijson::str(error)}}) << std::endl;
    std::cerr << "qi-dump-values: " << name << ": " << error << "\n";
  }

  template <typename T>
  void dump(const std::string& name, const T& value)
  {
    try
    {
      qi::Buffer buf;
      qi::encodeBinary(&buf, value);
      const std::string sig = qi::typeOf<T>()->signature().toString();
      emitValue(name, sig, buf, sig.find('r') == std::string::npos ? jsonOf(value) : std::string());
    }
    catch (const std::exception& e)
    {
      emitError(name, e.what());
    }
  }

  void dumpRef(const std::string& name, const qi::AnyReference& ref)
  {
    try
    {
      qi::Buffer buf;
      qi::encodeBinary(&buf, ref);
      const std::string sig = ref.type()->signature().toString();
      emitValue(name, sig, buf, sig.find('r') == std::string::npos ? jsonOf(ref) : std::string());
    }
    catch (const std::exception& e)
    {
      emitError(name, e.what());
    }
  }

  void dumpDynamic(const std::string& name, const qi::AnyValue& value)
  {
    dumpRef(name, qi::AnyReference::from(value));
  }

  /// qi::Message is not exported by libqimessaging 2.1 (hidden symbols), so
  /// messages are modelled here: the 28-byte header of src/message.hpp and a
  /// payload built with the same rules as Message::setValue/setValues/setError.
  struct WireMessage
  {
    enum Type { Type_None = 0, Type_Call = 1, Type_Reply = 2, Type_Error = 3, Type_Post = 4, Type_Event = 5, Type_Capability = 6 };
    static const std::uint8_t TypeFlag_DynamicPayload = 1;

    WireMessage() : id(0), version(0), type(Type_None), flags(0), service(0), object(0), action(0) {}
    std::uint32_t id;
    std::uint16_t version;
    std::uint8_t type;
    std::uint8_t flags;
    std::uint32_t service;
    std::uint32_t object;
    std::uint32_t action;
    qi::Buffer buffer;

    /// Message::setValue: the value, converted to the signature.
    void setValue(const qi::AutoAnyReference& value, const qi::Signature& sig)
    {
      if (value.type()->signature() == sig)
      {
        qi::encodeBinary(&buffer, value);
        return;
      }
      std::pair<qi::AnyReference, bool> conv = value.convert(qi::TypeInterface::fromSignature(sig));
      if (!conv.first.type())
        throw std::runtime_error("setValue: cannot convert to " + sig.toString());
      qi::encodeBinary(&buffer, conv.first);
      if (conv.second)
        conv.first.destroy();
    }
    /// Message::setValues: the values one after the other, or, for the
    /// signature "m", a dynamic holding the tuple of the values.
    void setValues(const qi::AnyReferenceVector& values, const qi::Signature& sig)
    {
      if (sig == qi::Signature("m"))
      {
        qi::AnyValue tuple = qi::AnyValue::makeTuple(values);
        qi::encodeBinary(&buffer, qi::AnyReference::from(tuple));
        return;
      }
      for (size_t i = 0; i < values.size(); ++i)
        qi::encodeBinary(&buffer, values[i]);
    }
    /// Message::setError: a dynamic holding the string.
    void setError(const std::string& error)
    {
      qi::AnyValue v = qi::AnyValue::from(error);
      qi::encodeBinary(&buffer, qi::AnyReference::from(v));
    }
  };

  const char* typeName(std::uint8_t type)
  {
    static const char* names[] = {"None", "Call", "Reply", "Error", "Post", "Event", "Capability"};
    return type < 7 ? names[type] : "Unknown";
  }

  template <typename T>
  void put(std::vector<std::uint8_t>& out, T v)
  {
    const std::uint8_t* p = reinterpret_cast<const std::uint8_t*>(&v);
    out.insert(out.end(), p, p + sizeof(T));
  }

  void dumpMessage(const std::string& name, const WireMessage& msg, const std::string& note = "")
  {
    const std::vector<std::uint8_t> payload = flatten(msg.buffer);
    std::vector<std::uint8_t> bytes;
    put<std::uint32_t>(bytes, 0x42adde42u);
    put<std::uint32_t>(bytes, msg.id);
    put<std::uint32_t>(bytes, static_cast<std::uint32_t>(payload.size()));
    put<std::uint16_t>(bytes, msg.version);
    put<std::uint8_t>(bytes, msg.type);
    put<std::uint8_t>(bytes, msg.flags);
    put<std::uint32_t>(bytes, msg.service);
    put<std::uint32_t>(bytes, msg.object);
    put<std::uint32_t>(bytes, msg.action);
    bytes.insert(bytes.end(), payload.begin(), payload.end());

    std::vector<std::pair<std::string, std::string> > fields;
    fields.push_back(std::make_pair("name", ijson::str(name)));
    fields.push_back(std::make_pair("kind", ijson::str("message")));
    fields.push_back(std::make_pair("id", ijson::num(msg.id)));
    fields.push_back(std::make_pair("type", ijson::num(static_cast<unsigned>(msg.type))));
    fields.push_back(std::make_pair("type_name", ijson::str(typeName(msg.type))));
    fields.push_back(std::make_pair("flags", ijson::num(static_cast<unsigned>(msg.flags))));
    fields.push_back(std::make_pair("service", ijson::num(msg.service)));
    fields.push_back(std::make_pair("object", ijson::num(msg.object)));
    fields.push_back(std::make_pair("action", ijson::num(msg.action)));
    fields.push_back(std::make_pair("payload_size", ijson::num(static_cast<unsigned>(payload.size()))));
    fields.push_back(std::make_pair("payload_hex", ijson::str(ijson::hex(payload))));
    fields.push_back(std::make_pair("hex", ijson::str(ijson::hex(bytes))));
    if (!note.empty())
      fields.push_back(std::make_pair("note", ijson::str(note)));
    std::cout << ijson::object(fields) << std::endl;
  }

  int addOne(int v) { return v + 1; }

  /// A stream context with chosen local and remote capabilities, to serialize
  /// object references the way a socket with those capabilities would.
  class TestContext : public qi::StreamContext
  {
  public:
    TestContext(const qi::CapabilityMap& local, const qi::CapabilityMap& remote)
    {
      _localCapabilityMap = local;
      _remoteCapabilityMap = remote;
    }
    virtual void advertiseCapabilities(const qi::CapabilityMap& map)
    {
      _localCapabilityMap.insert(map.begin(), map.end());
    }
  };

  qi::ObjectSerializationInfo serializeObject(const qi::AnyObject& obj)
  {
    qi::ObjectSerializationInfo osi;
    osi.metaObject = obj.metaObject();
    osi.serviceId = 2;
    osi.objectId = 3;
    return osi;
  }

  void dumpObject(const std::string& name, const qi::AnyObject& obj, qi::StreamContext* ctx, const std::string& note)
  {
    try
    {
      qi::Buffer buf;
      qi::encodeBinary(&buf, obj, &serializeObject, ctx);
      std::vector<std::pair<std::string, std::string> > fields;
      fields.push_back(std::make_pair("name", ijson::str(name)));
      fields.push_back(std::make_pair("signature", ijson::str("o")));
      fields.push_back(std::make_pair("hex", ijson::str(ijson::hex(flatten(buf)))));
      fields.push_back(std::make_pair("note", ijson::str(note)));
      std::cout << ijson::object(fields) << std::endl;
    }
    catch (const std::exception& e)
    {
      emitError(name, e.what());
    }
  }
}

int main(int argc, char** argv)
{
  qi::Application app(argc, argv);
  interop::routeQiLogsToStderr();

  // ---- primitives -------------------------------------------------------
  dump<bool>("bool_true", true);
  dump<bool>("bool_false", false);
  dump<std::int8_t>("int8_neg5", -5);
  dump<std::uint8_t>("uint8_200", 200);
  dump<std::int16_t>("int16_neg1234", -1234);
  dump<std::uint16_t>("uint16_60000", 60000);
  dump<std::int32_t>("int32_neg100000", -100000);
  dump<std::uint32_t>("uint32_3000000000", 3000000000u);
  dump<std::int64_t>("int64_neg5e12", -5000000000000LL);
  dump<std::uint64_t>("uint64_1e19", 10000000000000000000ULL);
  dump<float>("float_1_5", 1.5f);
  dump<double>("double_neg2_25", -2.25);

  // ---- strings ----------------------------------------------------------
  dump<std::string>("string_empty", std::string());
  dump<std::string>("string_abc", std::string("abc"));
  dump<std::string>("string_hello_utf8", std::string("h\xc3\xa9llo"));

  // ---- containers -------------------------------------------------------
  {
    std::vector<std::int32_t> v;
    v.push_back(1);
    v.push_back(2);
    v.push_back(3);
    dump("vec_int32_1_2_3", v);
    std::vector<std::string> vs;
    vs.push_back("a");
    vs.push_back("bc");
    dump("vec_string_a_bc", vs);
    dump("vec_int32_empty", std::vector<std::int32_t>());
    std::map<std::string, std::int32_t> msi;
    msi["a"] = 1;
    msi["b"] = 2;
    dump("map_string_int32", msi);
    std::map<std::int32_t, std::string> mis;
    mis[1] = "a";
    dump("map_int32_string", mis);
    const std::int32_t i = 1;
    const std::string s = "a";
    qi::AnyReferenceVector refs;
    refs.push_back(qi::AnyReference::from(i));
    refs.push_back(qi::AnyReference::from(s));
    qi::AnyValue tuple = qi::AnyValue::makeTuple(refs);
    dumpRef("tuple_int32_string", tuple.asReference());
    dump("pair_int32_string", std::pair<std::int32_t, std::string>(1, "a"));
  }

  // ---- registered structs -----------------------------------------------
  {
    Point2D p;
    p.x = 4;
    p.y = 2;
    dump("struct_point2d", p);
    TimeStampedPoint2D tp;
    tp.p = p;
    tp.t.i = 3;
    tp.t.j = 1;
    dump("struct_nested_timestamped_point2d", tp);
  }

  // ---- dynamics -----------------------------------------------------------
  {
    dumpDynamic("dynamic_int32_5", qi::AnyValue::from<std::int32_t>(5));
    dumpDynamic("dynamic_string_abc", qi::AnyValue::from<std::string>("abc"));
    std::vector<std::int32_t> v12;
    v12.push_back(1);
    v12.push_back(2);
    dumpDynamic("dynamic_vec_int32_1_2", qi::AnyValue::from(v12));
    Point2D p;
    p.x = 4;
    p.y = 2;
    dumpDynamic("dynamic_point2d", qi::AnyValue::from(p));
    dumpDynamic("dynamic_empty", qi::AnyValue());
    dumpDynamic("dynamic_void", qi::AnyValue(qi::typeOf<void>()));
    dumpDynamic("dynamic_bool_true", qi::AnyValue::from<bool>(true));
    dumpDynamic("dynamic_double_1_5", qi::AnyValue::from<double>(1.5));
    std::map<std::string, qi::AnyValue> msd;
    msd["i"] = qi::AnyValue::from<std::int32_t>(1);
    msd["s"] = qi::AnyValue::from<std::string>("x");
    dumpDynamic("dynamic_map_string_dynamic", qi::AnyValue::from(msd));
    std::vector<qi::AnyValue> vd;
    vd.push_back(qi::AnyValue::from<std::int32_t>(1));
    vd.push_back(qi::AnyValue::from<std::string>("a"));
    dump("vec_dynamic_int32_string", vd);
  }

  // ---- raw buffer -----------------------------------------------------------
  {
    qi::Buffer raw;
    const std::uint8_t bytes[4] = {0x2a, 0, 0, 0};
    raw.write(bytes, sizeof(bytes));
    dump("buffer_4_bytes", raw);
    qi::Buffer empty;
    dump("buffer_empty", empty);
    const std::int32_t seven = 7;
    const std::string z = "z";
    qi::AnyReferenceVector refs;
    refs.push_back(qi::AnyReference::from(seven));
    refs.push_back(qi::AnyReference::from(raw));
    refs.push_back(qi::AnyReference::from(z));
    qi::AnyValue tuple = qi::AnyValue::makeTuple(refs);
    dumpRef("tuple_int32_buffer_string", tuple.asReference());
  }

  // ---- misc registered types ------------------------------------------------
  dump("signature_value_is", qi::Signature("(is)"));
  {
    qi::os::timeval tv;
    tv.tv_sec = 1700000000;
    tv.tv_usec = 123456;
    dump("os_timeval", tv);
  }
  dump("url_tcp_localhost", qi::Url("tcp://127.0.0.1:9559"));

  // ---- MetaObject ---------------------------------------------------------
  qi::AnyObject testObject;
  {
    qi::DynamicObject* dyn = new qi::DynamicObject();
    qi::DynamicObjectBuilder ob(dyn, true);
    ob.advertiseMethod("add", &addOne, std::string("Adds one to its argument"));
    ob.advertiseSignal<int>("fire");
    ob.advertiseProperty<int>("val");
    ob.setDescription("Interop test object");
    const qi::MetaObject rawMo = dyn->metaObject();
    dump("metaobject_builder_raw", rawMo);
    testObject = ob.object();
    dump("metaobject_full_object", testObject.metaObject());
    dump("metaobject_empty", qi::MetaObject());
  }

  // ---- ServiceInfo (6 fields: no objectUid in 2.1) ------------------------
  {
    qi::ServiceInfo si;
    si.setName("Calculator");
    si.setServiceId(2);
    si.setMachineId("9a65b56e-c3d3-4485-8924-661b036202b3");
    si.setProcessId(3420486);
    qi::UrlVector eps;
    eps.push_back(qi::Url("tcp://127.0.0.1:41681"));
    si.setEndpoints(eps);
    si.setSessionId("361ecec4-00f7-4c94-a6e2-d91e28c5a06c");
    dump("serviceinfo_calculator", si);

    qi::ServiceInfo sdInfo;
    sdInfo.setName("ServiceDirectory");
    sdInfo.setServiceId(1);
    sdInfo.setMachineId("9a65b56e-c3d3-4485-8924-661b036202b3");
    sdInfo.setProcessId(3420486);
    qi::UrlVector sdEps;
    sdEps.push_back(qi::Url("tcp://127.0.0.1:9559"));
    sdInfo.setEndpoints(sdEps);
    sdInfo.setSessionId("0");
    dump("serviceinfo_servicedirectory_no_uid", sdInfo);
    std::vector<qi::ServiceInfo> infos;
    infos.push_back(si);
    dump("vec_serviceinfo_one", infos);
  }

  // ---- CapabilityMap --------------------------------------------------------
  const qi::CapabilityMap defaultCaps = qi::StreamContext::defaultCapabilities();
  dump("capabilitymap_default", defaultCaps);

  // ---- Object references ----------------------------------------------------
  {
    qi::CapabilityMap noCache;
    noCache["ClientServerSocket"] = qi::AnyValue::from(true);
    noCache["MessageFlags"] = qi::AnyValue::from(true);
    noCache["MetaObjectCache"] = qi::AnyValue::from(false);
    TestContext plain(defaultCaps, noCache);
    dumpObject("objectref_plain", testObject, &plain,
               "MetaObjectCache not shared: metaObject, serviceId=2, objectId=3");
    TestContext cached(defaultCaps, defaultCaps);
    dumpObject("objectref_cached_first", testObject, &cached,
               "MetaObjectCache shared, first transmission: bool transmit=1, metaObject, cacheId, serviceId, objectId");
    dumpObject("objectref_cached_second", testObject, &cached,
               "MetaObjectCache shared, same meta object again: bool transmit=0, cacheId, serviceId, objectId");
  }

  // ---- Messages -------------------------------------------------------------
  {
    // What a 2.1 server answers to the authentication call of a 2.3+ client
    // (Server::onMessageReady: no bound object for service 0): a Type_Error
    // with the id and address of the call.
    WireMessage err;
    err.type = WireMessage::Type_Error;
    err.id = 1;
    err.service = 0;
    err.object = 0;
    err.action = 8;
    err.setError("can't find service, address: {0.0.8, id:1}");
    dumpMessage("msg_error_cant_find_service", err,
                "reply of a 2.1 server to the authentication call {0.0.8, id:1}: Type_Error, same id and address");
  }
  {
    WireMessage err;
    err.type = WireMessage::Type_Error;
    err.id = 5;
    err.service = 2;
    err.object = 1;
    err.action = 100;
    err.setError("boom");
    dumpMessage("msg_error_boom", err, "error payload is a dynamic (m) holding the string");
  }
  {
    WireMessage event;
    event.type = WireMessage::Type_Event;
    event.id = 11;
    event.service = 2;
    event.object = 1;
    event.action = 100;
    const std::int32_t v = 42;
    qi::AnyReferenceVector refs;
    refs.push_back(qi::AnyReference::from(v));
    event.setValues(refs, qi::Signature("(i)"));
    dumpMessage("msg_event_int32_42", event, "payload = signal parameter tuple (i)");
  }
  {
    WireMessage call;
    call.type = WireMessage::Type_Call;
    call.id = 3;
    call.service = 2;
    call.object = 1;
    call.action = 100;
    const std::int32_t a = 1, b = 2;
    qi::AnyReferenceVector refs;
    refs.push_back(qi::AnyReference::from(a));
    refs.push_back(qi::AnyReference::from(b));
    call.setValues(refs, qi::Signature("(ii)"));
    dumpMessage("msg_call_add_1_2", call, "method call with argument tuple (ii)");
  }
  {
    WireMessage reply;
    reply.type = WireMessage::Type_Reply;
    reply.id = 3;
    reply.service = 2;
    reply.object = 1;
    reply.action = 100;
    const std::int32_t r = 3;
    reply.setValue(r, "i");
    dumpMessage("msg_reply_add_3", reply, "reply carries the call id and address");
  }
  {
    WireMessage call;
    call.type = WireMessage::Type_Call;
    call.id = 2;
    call.service = 2;
    call.object = 1;
    call.action = 2; // BoundObjectFunction_MetaObject
    const unsigned int zero = 0;
    qi::AnyReferenceVector refs;
    refs.push_back(qi::AnyReference::from(zero));
    call.setValues(refs, qi::Signature("(I)"));
    dumpMessage("msg_call_metaobject", call, "metaObject::(I) as sent by RemoteObject::fetchMetaObject (argument 0)");
  }
  {
    WireMessage call;
    call.type = WireMessage::Type_Call;
    call.id = 4;
    call.service = 2;
    call.object = 1;
    call.action = 0; // BoundObjectFunction_RegisterEvent
    const unsigned int service = 2, event = 100;
    const qi::uint64_t link = (static_cast<qi::uint64_t>(event) << 32) | 2u;
    qi::AnyReferenceVector refs;
    refs.push_back(qi::AnyReference::from(service));
    refs.push_back(qi::AnyReference::from(event));
    refs.push_back(qi::AnyReference::from(link));
    call.setValues(refs, qi::Signature("(IIL)"));
    dumpMessage("msg_call_registerevent", call, "registerEvent::(IIL) -> L");
  }
  {
    WireMessage post;
    post.type = WireMessage::Type_Post;
    post.id = 6;
    post.service = 2;
    post.object = 1;
    post.action = 101;
    const std::string s = "abc";
    qi::AnyReferenceVector refs;
    refs.push_back(qi::AnyReference::from(s));
    post.setValues(refs, qi::Signature("(s)"));
    dumpMessage("msg_post_string_abc", post, "post (no reply expected) with argument tuple (s)");
  }
  {
    WireMessage dyn;
    dyn.type = WireMessage::Type_Call;
    dyn.id = 8;
    dyn.service = 2;
    dyn.object = 1;
    dyn.action = 100;
    dyn.flags |= WireMessage::TypeFlag_DynamicPayload;
    const std::int32_t a = 1;
    const std::string b = "a";
    qi::AnyReferenceVector refs;
    refs.push_back(qi::AnyReference::from(a));
    refs.push_back(qi::AnyReference::from(b));
    dyn.setValues(refs, qi::Signature("m"));
    dumpMessage("msg_call_dynamic_payload", dyn, "TypeFlag_DynamicPayload: payload is a dynamic wrapping the argument tuple (is)");
  }
  {
    // TcpTransportSocket::advertiseCapabilities: sent by both ends right after
    // the connection, before anything else (service/object/action left at 0).
    WireMessage caps;
    caps.type = WireMessage::Type_Capability;
    caps.id = 10;
    caps.setValue(defaultCaps, qi::typeOf<qi::CapabilityMap>()->signature());
    dumpMessage("msg_capability_default", caps, "Type_Capability message sent on connection by both ends (2.1 defaults)");
  }

  std::cout.flush();
  return 0;
}
