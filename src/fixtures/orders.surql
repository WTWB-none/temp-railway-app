UPSERT orders:u'018f1000-0000-7000-8000-000000003001' CONTENT {
    products: [
        [products:u'018f1000-0000-7000-8000-000000001001', 2],
        [products:u'018f1000-0000-7000-8000-000000001003', 4]
    ],
    package_box: [],
    status: { NotPacked: {} }
};

UPSERT orders:u'018f1000-0000-7000-8000-000000003002' CONTENT {
    products: [
        [products:u'018f1000-0000-7000-8000-000000001002', 5],
        [products:u'018f1000-0000-7000-8000-000000001004', 3]
    ],
    package_box: [
        [boxes:u'018f1000-0000-7000-8000-000000002002', 1]
    ],
    status: { Packed: {} }
};
